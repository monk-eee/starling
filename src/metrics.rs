use std::{collections::BTreeMap, time::Duration};

use hdrhistogram::Histogram;
use serde::{Serialize, Serializer};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Outcome {
    Success,
    Transport,
    Protocol,
    ProtocolVersion,
    RpcError,
    Capability,
    ToolError,
    Assertion,
    Timeout,
    Cancelled,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Transport => "transport",
            Self::Protocol => "protocol",
            Self::ProtocolVersion => "protocol_version",
            Self::RpcError => "json_rpc",
            Self::Capability => "capability",
            Self::ToolError => "tool_error",
            Self::Assertion => "assertion",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
        }
    }
}

impl Serialize for Outcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

struct Stats {
    latency_us: Histogram<u64>,
    outcomes: BTreeMap<Outcome, u64>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            latency_us: Histogram::new(3).expect("valid HDR precision"),
            outcomes: BTreeMap::new(),
        }
    }
}

impl Stats {
    fn record(&mut self, latency: Duration, outcome: Outcome) {
        self.latency_us
            .record(latency.as_micros() as u64)
            .expect("auto-resizing HDR histogram");
        *self.outcomes.entry(outcome).or_default() += 1;
    }

    fn merge(&mut self, other: &Self) {
        self.latency_us
            .add(&other.latency_us)
            .expect("matching HDR precision");
        for (&outcome, &count) in &other.outcomes {
            *self.outcomes.entry(outcome).or_default() += count;
        }
    }

    fn summarize(&self) -> Summary {
        let count = self.latency_us.len();
        let quantile =
            |q| (count != 0).then(|| self.latency_us.value_at_quantile(q) as f64 / 1000.0);
        Summary {
            count,
            failed: count - self.outcomes.get(&Outcome::Success).copied().unwrap_or(0),
            p50_ms: quantile(0.50),
            p95_ms: quantile(0.95),
            p99_ms: quantile(0.99),
            outcomes: self.outcomes.clone(),
        }
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Summary {
    pub count: u64,
    pub failed: u64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub outcomes: BTreeMap<Outcome, u64>,
}

#[derive(Clone, Serialize)]
pub struct Point {
    pub second: u64,
    pub span_ms: f64,
    pub calls: u64,
    pub failed: u64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
}

pub struct Metrics {
    operations: BTreeMap<String, Stats>,
    windows: BTreeMap<u64, Stats>,
    bucket_secs: u64,
    planned_buckets: u64,
    pub series: Vec<Point>,
}

impl Metrics {
    const POINT_BUDGET: u64 = 1800;

    fn is_workload(key: &str) -> bool {
        key.starts_with("tools/call/") || key.starts_with("task/")
    }

    pub fn bucket_secs(duration: Duration) -> u64 {
        let seconds = duration.as_secs() + u64::from(duration.subsec_nanos() != 0);
        seconds.div_ceil(Self::POINT_BUDGET).max(1)
    }

    pub fn new(duration: Duration) -> Self {
        let bucket_secs = Self::bucket_secs(duration);
        let seconds = duration.as_secs() + u64::from(duration.subsec_nanos() != 0);
        Self {
            operations: BTreeMap::new(),
            windows: BTreeMap::new(),
            series: Vec::new(),
            bucket_secs,
            planned_buckets: seconds.div_ceil(bucket_secs),
        }
    }

    fn index(&self, at: Duration) -> u64 {
        // A delayed wake or a long drain gets one overflow bucket, not hours of empty ones.
        (at.as_secs() / self.bucket_secs).min(self.planned_buckets)
    }

    pub fn record(&mut self, key: &str, at: Duration, latency: Duration, outcome: Outcome) {
        self.operations
            .entry(key.to_owned())
            .or_default()
            .record(latency, outcome);
        if Self::is_workload(key) {
            let index = self.index(at);
            self.windows
                .entry(index)
                .or_default()
                .record(latency, outcome);
        }
    }

    pub fn summarize(&self) -> (Summary, BTreeMap<String, Summary>) {
        let mut calls = Stats::default();
        let operations = self
            .operations
            .iter()
            .map(|(key, stats)| {
                if Self::is_workload(key) {
                    calls.merge(stats);
                }
                (key.clone(), stats.summarize())
            })
            .collect();
        (calls.summarize(), operations)
    }

    pub fn seal(&mut self, elapsed: Duration, finish: bool) {
        let full = self.index(elapsed);
        while (self.series.len() as u64) < full {
            self.seal_next(Duration::from_secs(self.bucket_secs));
        }
        let partial = elapsed - Duration::from_secs(full * self.bucket_secs);
        if finish && (!partial.is_zero() || self.windows.contains_key(&full)) {
            self.seal_next(partial.max(Duration::from_nanos(1)));
        }
    }

    pub fn open_interval(&self, elapsed: Duration) -> Option<Point> {
        let index = self.index(elapsed);
        let second = index * self.bucket_secs;
        let span = elapsed - Duration::from_secs(second);
        (self.bucket_secs > 1 && span >= Duration::from_secs(1))
            .then(|| Self::point(second, span, self.windows.get(&index)))
    }

    fn seal_next(&mut self, span: Duration) {
        let index = self.series.len() as u64;
        let stats = self.windows.remove(&index);
        self.series
            .push(Self::point(index * self.bucket_secs, span, stats.as_ref()));
    }

    fn point(second: u64, span: Duration, stats: Option<&Stats>) -> Point {
        let s = stats.map(Stats::summarize).unwrap_or_default();
        Point {
            second,
            span_ms: span.as_secs_f64() * 1000.0,
            calls: s.count,
            failed: s.failed,
            p50_ms: s.p50_ms,
            p95_ms: s.p95_ms,
            p99_ms: s.p99_ms,
        }
    }
}

#[cfg(test)]
mod tests;
