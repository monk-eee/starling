use std::{
    env,
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use tokio::sync::oneshot;

use crate::runner::Status;
#[cfg(feature = "otlp")]
use crate::{config::Method, metrics::Outcome};

#[cfg(feature = "otlp")]
mod otlp;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Serialize)]
struct Totals {
    status: &'static str,
    requests: u64,
    failed: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    timestamp_ms: i128,
    service: &'static str,
    level: Level,
    event: &'static str,
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bird_id: Option<String>,
    #[serde(flatten)]
    totals: Option<Totals>,
    #[cfg(feature = "otlp")]
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<String>,
    #[cfg(feature = "otlp")]
    #[serde(skip_serializing_if = "Option::is_none")]
    span_id: Option<String>,
}

#[derive(Default)]
pub struct RunTrace {
    #[cfg(feature = "otlp")]
    context: Option<opentelemetry::Context>,
}

pub struct Telemetry {
    role: &'static str,
    bird: Option<String>,
    minimum: Option<Level>,
    stderr: Option<mpsc::SyncSender<Record>>,
    dropped: AtomicU64,
    #[cfg(feature = "otlp")]
    export: Option<Arc<otlp::Export>>,
}

pub struct Logging {
    pub events: Arc<Telemetry>,
    drained: Option<oneshot::Receiver<io::Result<()>>>,
}

fn setting(key: &str) -> Result<Option<String>> {
    env::var_os(key)
        .map(|s| {
            s.into_string()
                .map_err(|_| anyhow::anyhow!("{key} must be UTF-8"))
        })
        .transpose()
}

impl Logging {
    pub async fn open(role: &'static str, bird: Option<&str>, active: bool) -> Result<Self> {
        let selection = if active {
            setting("STARLING_TELEMETRY")?
        } else {
            Some("none".into())
        };
        let (local, remote) = match selection.as_deref().unwrap_or("stderr") {
            "none" => (false, false),
            "stderr" => (true, false),
            "otlp" => (false, true),
            "stderr,otlp" | "otlp,stderr" => (true, true),
            _ => bail!("STARLING_TELEMETRY must be stderr, otlp, stderr,otlp or none"),
        };
        let minimum = if local || remote {
            match setting("STARLING_LOG_LEVEL")?.as_deref().unwrap_or("info") {
                "info" => Some(Level::Info),
                "warn" => Some(Level::Warn),
                "error" => Some(Level::Error),
                "off" => None,
                _ => bail!("STARLING_LOG_LEVEL must be info, warn, error or off"),
            }
        } else {
            None
        };
        #[cfg(not(feature = "otlp"))]
        ensure!(
            !remote,
            "OTLP export was selected but this binary was built without --features otlp"
        );
        #[cfg(feature = "otlp")]
        let export = if remote {
            let endpoint = setting("OTEL_EXPORTER_OTLP_ENDPOINT")?
                .context("OTLP export requires OTEL_EXPORTER_OTLP_ENDPOINT")?;
            let bird = bird.map(str::to_owned);
            Some(Arc::new(
                tokio::task::spawn_blocking(move || otlp::Export::open(&endpoint, role, bird))
                    .await??,
            ))
        } else {
            None
        };
        let (stderr, drained) = if local && minimum.is_some() {
            let (send, receive) = mpsc::sync_channel::<Record>(256);
            let (done, drained) = oneshot::channel();
            std::thread::Builder::new()
                .name("starling-stderr".into())
                .spawn(move || {
                    let result = receive.into_iter().try_for_each(|row| -> io::Result<()> {
                        let mut line = serde_json::to_vec(&row)?;
                        line.push(b'\n');
                        io::stderr().lock().write_all(&line)
                    });
                    let _ = done.send(result);
                })
                .context("starting the structured log writer")?;
            (Some(send), Some(drained))
        } else {
            (None, None)
        };
        Ok(Self {
            events: Arc::new(Telemetry {
                role,
                bird: bird.map(str::to_owned),
                minimum,
                stderr,
                dropped: AtomicU64::new(0),
                #[cfg(feature = "otlp")]
                export,
            }),
            drained,
        })
    }

    pub async fn shutdown(self) -> Result<()> {
        let dropped = self.events.dropped.load(Ordering::Relaxed);
        #[cfg(feature = "otlp")]
        let export = self.events.export.clone();
        drop(self.events);
        let local = async {
            if let Some(done) = self.drained {
                tokio::time::timeout(Duration::from_secs(1), done)
                    .await
                    .context("timed out draining structured stderr")?
                    .context("structured log writer terminated")?
                    .context("writing structured stderr")?;
            }
            ensure!(dropped == 0, "structured stderr dropped {dropped} records");
            Ok(())
        };
        #[cfg(feature = "otlp")]
        {
            let remote = async {
                if let Some(export) = export {
                    tokio::task::spawn_blocking(move || export.close()).await??;
                }
                Ok::<_, anyhow::Error>(())
            };
            let (local, remote) = tokio::join!(local, remote);
            local.and(remote)
        }
        #[cfg(not(feature = "otlp"))]
        local.await
    }
}

impl Telemetry {
    pub fn event(&self, level: Level, name: &'static str) {
        self.emit(level, name, None, &RunTrace::default());
    }

    fn emit(&self, level: Level, event: &'static str, totals: Option<Totals>, _trace: &RunTrace) {
        if self.minimum.is_none_or(|minimum| level < minimum) {
            return;
        }
        let now = SystemTime::now();
        #[cfg(feature = "otlp")]
        let ids = self
            .export
            .as_ref()
            .and_then(|export| export.log(now, level, event, totals, _trace));
        if let Some(stderr) = &self.stderr {
            let row = Record {
                timestamp_ms: now
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i128)
                    .unwrap_or_else(|before| -(before.duration().as_millis() as i128)),
                service: "starling",
                level,
                event,
                role: self.role,
                bird_id: self.bird.clone(),
                totals,
                #[cfg(feature = "otlp")]
                trace_id: ids.as_ref().map(|(trace, _)| trace.to_string()),
                #[cfg(feature = "otlp")]
                span_id: ids.map(|(_, span)| span.to_string()),
            };
            if stderr.try_send(row).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn start_run(&self) -> RunTrace {
        let trace = RunTrace {
            #[cfg(feature = "otlp")]
            context: self.export.as_ref().map(|x| x.start_run()),
        };
        self.emit(Level::Info, "run_started", None, &trace);
        trace
    }

    pub fn finish_run(&self, trace: RunTrace, status: Status, requests: u64, failed: u64) {
        let totals = Totals {
            status: status.label(),
            requests,
            failed,
        };
        let level = if status == Status::Failed {
            Level::Error
        } else {
            Level::Info
        };
        self.emit(level, "run_finished", Some(totals), &trace);
        #[cfg(feature = "otlp")]
        trace.finish(status);
    }

    #[cfg(feature = "otlp")]
    pub fn request(&self, method: Method, outcome: Outcome, latency: Duration) {
        if let Some(export) = &self.export {
            export.request(method, outcome, latency);
        }
    }
}
