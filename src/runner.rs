use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::{
    sync::watch,
    task::JoinSet,
    time::{self, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{Config, MAX_DURATION, Protocol, StartOptions},
    mcp::Session,
    metrics::{Metrics, Outcome, Point, Summary},
    report,
};

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    Running,
    Stopping,
    Completed,
    Stopped,
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Completed => "completed",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Stopped | Self::Failed)
    }
}

#[derive(Clone, Serialize)]
pub struct RunInfo {
    pub name: String,
    pub target_origin: String,
    pub endpoint: String,
    pub endpoint_names: Vec<String>,
    pub duration_ms: u64,
    pub max_duration_ms: u64,
    pub bucket_secs: u64,
    pub users: u32,
    pub threads: usize,
    pub wait_ms: u64,
    pub timeout_ms: u64,
    pub seed: u64,
    pub requests: Vec<String>,
}

impl From<&Config> for RunInfo {
    fn from(cfg: &Config) -> Self {
        Self {
            name: cfg.run.name.clone(),
            target_origin: cfg.target.endpoint().origin().ascii_serialization(),
            endpoint: cfg
                .target
                .selected
                .as_deref()
                .unwrap_or("default")
                .to_owned(),
            endpoint_names: cfg.target.endpoints.keys().cloned().collect(),
            duration_ms: cfg.run.duration.as_millis() as u64,
            max_duration_ms: MAX_DURATION.as_millis() as u64,
            bucket_secs: Metrics::bucket_secs(cfg.run.duration),
            users: cfg.load.users,
            threads: cfg.load.threads,
            wait_ms: cfg.load.wait.as_millis() as u64,
            timeout_ms: cfg.load.timeout.as_millis() as u64,
            seed: cfg.run.seed,
            requests: cfg.tasks.iter().map(|t| t.operation()).collect(),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct Overview {
    pub schema_version: u32,
    pub sequence: u64,
    pub run: RunInfo,
    pub status: Status,
    pub reason: Option<String>,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_interval: Option<Point>,
    pub sessions: u32,
    pub in_flight: u32,
    pub protocols: BTreeSet<String>,
    pub calls: Summary,
    pub operations: BTreeMap<String, Summary>,
}

#[derive(Clone, Serialize)]
pub struct Frame {
    #[serde(flatten)]
    pub view: Overview,
    pub series: Vec<Point>,
}

struct State {
    status: Status,
    started: Option<Instant>,
    stopped_by_user: bool,
    failure: Option<String>,
    sessions: u32,
    in_flight: u32,
    protocols: BTreeSet<String>,
    sequence: u64,
    metrics: Metrics,
}

impl State {
    fn elapsed(&self) -> Duration {
        self.started.map_or(Duration::ZERO, |t| t.elapsed())
    }

    fn frame(&mut self, info: &RunInfo, finish: bool) -> Frame {
        let elapsed = self.elapsed();
        self.metrics.seal(elapsed, finish);
        let (calls, operations) = self.metrics.summarize();
        self.sequence += 1;
        Frame {
            view: Overview {
                schema_version: 1,
                sequence: self.sequence,
                run: info.clone(),
                status: self.status,
                reason: self.failure.clone(),
                elapsed_ms: elapsed.as_millis() as u64,
                open_interval: if finish {
                    None
                } else {
                    self.metrics.open_interval(elapsed)
                },
                sessions: self.sessions,
                in_flight: self.in_flight,
                protocols: self.protocols.clone(),
                calls,
                operations,
            },
            series: self.metrics.series.clone(),
        }
    }
}

pub struct Run {
    defaults: Config,
    active: OnceLock<Config>,
    state: Mutex<State>,
    feed: watch::Sender<Arc<Frame>>,
    start_signal: CancellationToken,
    stop_signal: CancellationToken,
}

impl Run {
    pub fn new(cfg: Config) -> Arc<Self> {
        let info = RunInfo::from(&cfg);
        let mut state = State {
            status: Status::Ready,
            started: None,
            stopped_by_user: false,
            failure: None,
            sessions: 0,
            in_flight: 0,
            protocols: BTreeSet::new(),
            sequence: 0,
            metrics: Metrics::new(cfg.run.duration),
        };
        let (feed, _) = watch::channel(Arc::new(state.frame(&info, false)));
        Arc::new(Self {
            defaults: cfg,
            active: OnceLock::new(),
            state: Mutex::new(state),
            feed,
            start_signal: CancellationToken::new(),
            stop_signal: CancellationToken::new(),
        })
    }

    pub fn config(&self) -> &Config {
        self.active.get().unwrap_or(&self.defaults)
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<Frame>> {
        self.feed.subscribe()
    }

    pub fn snapshot(&self) -> Arc<Frame> {
        self.feed.borrow().clone()
    }

    fn publish(&self, state: &mut State) {
        self.feed
            .send_replace(Arc::new(state.frame(&self.config().into(), false)));
    }

    pub fn start(&self, options: StartOptions) -> Result<bool> {
        let mut state = self.state.lock().unwrap();
        if state.status != Status::Ready {
            return Ok(false);
        }
        let cfg = self.defaults.clone().with_options(options)?;
        state.metrics = Metrics::new(cfg.run.duration);
        assert!(self.active.set(cfg).is_ok(), "ready run already configured");
        state.status = Status::Running;
        state.started = Some(Instant::now());
        self.publish(&mut state);
        self.start_signal.cancel();
        Ok(true)
    }

    pub fn stop(&self) -> bool {
        self.halt(true, None)
    }

    fn halt(&self, by_user: bool, failure: Option<String>) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.status.terminal() {
            return false;
        }
        if state.failure.is_none() && failure.is_some() {
            state.failure = failure;
        } else if state.status == Status::Stopping {
            return false;
        }
        state.status = Status::Stopping;
        state.stopped_by_user |= by_user;
        self.publish(&mut state);
        self.stop_signal.cancel();
        self.start_signal.cancel();
        true
    }

    async fn measured<T>(
        &self,
        key: &str,
        op: impl Future<Output = Result<T, Outcome>>,
    ) -> Option<Result<T, Outcome>> {
        let start = {
            let mut s = self.state.lock().unwrap();
            if s.status != Status::Running || s.elapsed() >= self.config().run.duration {
                return None;
            }
            s.in_flight += 1;
            Instant::now()
        };
        let result = time::timeout(self.config().load.timeout, op)
            .await
            .unwrap_or(Err(Outcome::Timeout));
        let latency = start.elapsed();
        let mut s = self.state.lock().unwrap();
        s.in_flight -= 1;
        let at = s.elapsed();
        s.metrics.record(
            key,
            at,
            latency,
            result.as_ref().err().copied().unwrap_or(Outcome::Success),
        );
        Some(result)
    }

    async fn worker(&self, id: u32) {
        let cfg = self.config();
        let startup = match cfg.target.protocol {
            Protocol::Initialize => "initialize",
            Protocol::Auto => "session/startup",
        };
        let session = match self
            .measured(
                startup,
                Session::open(&cfg.target, &cfg.tasks, cfg.load.timeout),
            )
            .await
        {
            None => return,
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                self.halt(false, Some(format!("{startup} failed: {}", e.label())));
                return;
            }
        };
        {
            let mut s = self.state.lock().unwrap();
            s.sessions += 1;
            s.protocols.insert(session.version());
        }
        let discovery = if cfg.tasks.iter().any(|t| t.tool.is_some()) {
            self.measured("tools/list", session.discover(&cfg.tasks))
                .await
        } else {
            Some(Ok(()))
        };
        match discovery {
            Some(Ok(())) => self.call_loop(id, &session).await,
            Some(Err(e)) => {
                self.halt(false, Some(format!("tools/list failed: {}", e.label())));
            }
            None => (),
        }
        let result = session.close(cfg.load.timeout).await;
        self.state.lock().unwrap().sessions -= 1;
        if let Err(e) = result {
            self.halt(false, Some(format!("session close failed: {}", e.label())));
        }
    }

    async fn call_loop(&self, id: u32, session: &Session) {
        let cfg = self.config();
        let mut rng = fastrand::Rng::with_seed(cfg.run.seed.wrapping_add(u64::from(id)));
        loop {
            let task = cfg.pick(&mut rng);
            let key = task.operation();
            match self.measured(&key, session.call(task)).await {
                None => break,
                Some(Err(e @ (Outcome::Transport | Outcome::Timeout))) => {
                    self.halt(false, Some(format!("{} failed: {}", key, e.label())));
                    break;
                }
                _ => (),
            }
            tokio::select! {
                _ = self.stop_signal.cancelled() => break,
                _ = time::sleep(cfg.load.wait) => (),
            }
        }
    }

    async fn drive(self: &Arc<Self>, start: Instant) {
        let mut workers = JoinSet::new();
        for id in 0..self.config().load.users {
            let run = self.clone();
            workers.spawn(async move { run.worker(id).await });
        }
        let start = time::Instant::from_std(start);
        let second = Duration::from_secs(1);
        let mut timer = time::interval_at(start + second, second);
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        while !workers.is_empty() {
            tokio::select! {
                _ = timer.tick() => self.publish(&mut self.state.lock().unwrap()),
                _ = time::sleep_until(start + self.config().run.duration), if !self.stop_signal.is_cancelled() => {
                    self.halt(false, None);
                }
                task = workers.join_next() => if let Some(Err(_)) = task {
                    self.halt(false, Some("worker terminated unexpectedly".into()));
                }
            }
        }
    }

    pub async fn execute(self: Arc<Self>, directory: PathBuf) -> Result<Arc<Frame>> {
        self.start_signal.cancelled().await;
        let start = self.state.lock().unwrap().started;
        if let Some(start) = start {
            let run = self.clone();
            // Create and drop the workload runtime outside the controller's async context.
            let result = tokio::task::spawn_blocking(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(run.config().load.threads)
                    .thread_name("starling-load")
                    .enable_all()
                    .build()?;
                runtime.block_on(run.drive(start));
                Ok::<_, std::io::Error>(())
            })
            .await;
            match result {
                Ok(Ok(())) => (),
                Ok(Err(_)) => {
                    self.halt(false, Some("could not create workload threads".into()));
                }
                Err(_) => {
                    self.halt(
                        false,
                        Some("workload runtime terminated unexpectedly".into()),
                    );
                }
            }
        }
        let mut frame = {
            let mut s = self.state.lock().unwrap();
            let (calls, _) = s.metrics.summarize();
            if s.failure.is_none() {
                s.failure = if calls.failed > 0 {
                    Some("one or more workload requests failed".into())
                } else if !s.stopped_by_user && calls.count == 0 {
                    Some("no workload requests completed before the deadline".into())
                } else {
                    None
                };
            }
            s.status = if s.failure.is_some() {
                Status::Failed
            } else if s.stopped_by_user {
                Status::Stopped
            } else {
                Status::Completed
            };
            s.frame(&self.config().into(), true)
        };
        // Publish terminal success only after the offline report is readable.
        let frame = match report::write(&directory, &frame) {
            Ok(()) => Arc::new(frame),
            Err(error) => {
                frame.view.status = Status::Failed;
                frame.view.reason = Some("report persistence failed".into());
                self.feed.send_replace(Arc::new(frame));
                return Err(error).context("writing report bundle");
            }
        };
        self.feed.send_replace(frame.clone());
        Ok(frame)
    }
}
