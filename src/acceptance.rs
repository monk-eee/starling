use std::{fmt, num::NonZeroU64, str::FromStr};

use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Number;

use crate::metrics::Summary;

// Missing rules are optional; an explicitly null rule must not silently disable a gate.
pub(crate) fn specified<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Criteria {
    #[serde(
        default,
        deserialize_with = "specified",
        skip_serializing_if = "Option::is_none"
    )]
    error_rate: Option<Upper<Percent>>,
    #[serde(
        default,
        deserialize_with = "specified",
        skip_serializing_if = "Option::is_none"
    )]
    p95: Option<Upper<humantime::Duration>>,
    #[serde(
        default,
        deserialize_with = "specified",
        skip_serializing_if = "Option::is_none"
    )]
    p99: Option<Upper<humantime::Duration>>,
    #[serde(
        default,
        deserialize_with = "specified",
        skip_serializing_if = "Option::is_none"
    )]
    min_requests: Option<NonZeroU64>,
}

#[derive(Clone, Copy)]
struct Upper<T> {
    inclusive: bool,
    bound: T,
}

impl<T> Upper<T> {
    fn allows<N: PartialOrd>(&self, actual: N, bound: N) -> bool {
        if self.inclusive {
            actual <= bound
        } else {
            actual < bound
        }
    }
}

impl<'de, T: FromStr> Deserialize<'de> for Upper<T>
where
    T::Err: fmt::Display,
{
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let rest = text
            .trim()
            .strip_prefix('<')
            .ok_or_else(|| de::Error::custom("expected < or <="))?;
        let (inclusive, value) = rest.strip_prefix('=').map_or((false, rest), |s| (true, s));
        let value = value.trim();
        if value.ends_with(|c: char| c.is_ascii_digit()) {
            return Err(de::Error::custom("an explicit unit is required"));
        }
        Ok(Self {
            inclusive,
            bound: value.parse().map_err(de::Error::custom)?,
        })
    }
}

impl<T: fmt::Display> fmt::Display for Upper<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}",
            if self.inclusive { "<=" } else { "<" },
            self.bound
        )
    }
}

impl<T: fmt::Display> Serialize for Upper<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[derive(Clone, Copy)]
struct Percent(u32);

impl Percent {
    const SCALE: u32 = 1_000_000;
}

impl FromStr for Percent {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const ERROR: &str = "expected 0-100% with at most six decimal places";
        let n = s.strip_suffix('%').ok_or(ERROR)?.trim_end();
        let (whole, frac) = n.split_once('.').unwrap_or((n, ""));
        if !whole.bytes().all(|b| b.is_ascii_digit())
            || !frac.bytes().all(|b| b.is_ascii_digit())
            || frac.len() > 6
            || n.ends_with('.')
        {
            return Err(ERROR);
        }
        let whole: u32 = whole.parse().map_err(|_| ERROR)?;
        if whole > 100 {
            return Err(ERROR);
        }
        let fraction = frac.bytes().fold(0u32, |v, b| v * 10 + u32::from(b - b'0'));
        let scaled = whole * Self::SCALE + fraction * 10u32.pow(6 - frac.len() as u32);
        (scaled <= 100 * Self::SCALE)
            .then_some(Self(scaled))
            .ok_or(ERROR)
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", f64::from(self.0) / f64::from(Self::SCALE))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Passed,
    Failed,
    Incomplete,
}

impl GateStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Clone, Serialize)]
pub struct Check {
    pub metric: &'static str,
    expected: String,
    observed: Option<Number>,
    unit: &'static str,
    pub passed: Option<bool>,
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: expected {}, observed ", self.metric, self.expected)?;
        match &self.observed {
            Some(n) => write!(f, "{n} {}", self.unit),
            None => f.write_str("no samples"),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct GateResult {
    pub status: GateStatus,
    pub checks: Vec<Check>,
}

impl GateResult {
    pub fn failure(&self) -> Option<String> {
        match self.status {
            GateStatus::Passed => None,
            GateStatus::Failed => Some(format!(
                "performance gate failed: {}",
                self.checks
                    .iter()
                    .filter(|c| c.passed == Some(false))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            GateStatus::Incomplete => Some(
                "performance gate incomplete: a finished workload with request samples is required"
                    .into(),
            ),
        }
    }
}

impl Criteria {
    pub fn from_map<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        struct Rules;
        impl<'de> de::Visitor<'de> for Rules {
            type Value = Criteria;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a mapping of performance assertions")
            }
            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Criteria, A::Error> {
                Criteria::deserialize(de::value::MapAccessDeserializer::new(map))
            }
        }
        d.deserialize_map(Rules).map(Some)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.error_rate.is_some()
                || self.p95.is_some()
                || self.p99.is_some()
                || self.min_requests.is_some(),
            "assert requires at least one of error_rate, p95, p99 or min_requests"
        );
        Ok(())
    }

    pub fn evaluate(&self, calls: &Summary, finished: bool) -> GateResult {
        let complete = finished && calls.count != 0;
        let mut checks = Vec::new();
        let mut record = |metric, expected, observed: Option<Number>, unit, met| {
            let passed = (complete && observed.is_some()).then_some(met);
            checks.push(Check {
                metric,
                expected,
                observed,
                unit,
                passed,
            });
        };
        if let Some(limit) = self.error_rate {
            // Compare integer ratios, not the percentage rounded for human display.
            let met = limit.allows(
                u128::from(calls.failed) * 100 * u128::from(Percent::SCALE),
                u128::from(calls.count) * u128::from(limit.bound.0),
            );
            let observed =
                (calls.count != 0).then(|| 100.0 * calls.failed as f64 / calls.count as f64);
            record(
                "error_rate",
                limit.to_string(),
                observed.and_then(Number::from_f64),
                "%",
                met,
            );
        }
        for (metric, limit, ms) in [
            ("p95", self.p95, calls.p95_ms),
            ("p99", self.p99, calls.p99_ms),
        ] {
            if let Some(limit) = limit {
                let met = ms
                    .is_some_and(|v| limit.allows(v, limit.bound.as_nanos() as f64 / 1_000_000.0));
                record(
                    metric,
                    limit.to_string(),
                    ms.and_then(Number::from_f64),
                    "ms",
                    met,
                );
            }
        }
        if let Some(min) = self.min_requests {
            record(
                "min_requests",
                format!(">= {min}"),
                Some(calls.count.into()),
                "requests",
                calls.count >= min.get(),
            );
        }
        let status = if checks.iter().any(|c| c.passed.is_none()) {
            GateStatus::Incomplete
        } else if checks.iter().any(|c| c.passed == Some(false)) {
            GateStatus::Failed
        } else {
            GateStatus::Passed
        };
        GateResult { status, checks }
    }
}
