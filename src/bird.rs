use std::{
    future::pending,
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use tokio::{
    net::TcpListener,
    process::Command,
    task::JoinSet,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tonic::{
    Code,
    transport::{Channel, ClientTlsConfig, Endpoint},
};
use url::{Host, Url};

use crate::{
    config::{Config, Protocol, StartOptions, cli_millis},
    fixture,
    flock::{
        Credentials, bird_id,
        rpc::{Heartbeat, telemetry_client::TelemetryClient},
    },
    report,
    runner::Run,
    telemetry::{Level, Telemetry},
};

#[derive(Args)]
pub struct Bird {
    /// Public, unique name shown in the flock.
    #[arg(long, value_parser = bird_id)]
    pub(super) id: String,
    /// Controller's telemetry URL (not the dashboard URL).
    #[arg(long)]
    home: Url,
    /// Run one locally supplied workload. Without this or --demo the bird is idle.
    #[arg(long, conflicts_with = "demo")]
    config: Option<PathBuf>,
    /// Select a named endpoint from the locally supplied workload.
    #[arg(long, requires = "config")]
    endpoint: Option<String>,
    /// Require an MCP revision for this bird's workload, or "auto" to negotiate.
    #[arg(long, value_name = "REVISION")]
    protocol: Option<Protocol>,
    /// Override this bird's per-user pause; 0 disables it.
    #[arg(long = "wait", value_name = "TIME", value_parser = cli_millis, allow_negative_numbers = true)]
    wait_ms: Option<u64>,
    /// Exercise only this bird's own ephemeral local fixture.
    #[arg(long)]
    demo: bool,
    #[arg(long, hide = true, requires = "demo")]
    supervised: bool,
    /// A new directory for the local report.
    #[arg(long)]
    report: Option<PathBuf>,
}

pub(crate) fn channel(home: &Url) -> Result<Channel> {
    ensure!(
        home.host().is_some()
            && home.username().is_empty()
            && home.password().is_none()
            && home.path() == "/"
            && home.query().is_none()
            && home.fragment().is_none(),
        "--home must be a telemetry origin without credentials, path, query, or fragment"
    );
    let local = matches!(home.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(home.host(), Some(Host::Ipv6(ip)) if ip.is_loopback());
    let endpoint = Endpoint::from_shared(home.origin().ascii_serialization())?
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2));
    let endpoint = match (home.scheme(), Credentials::from_env()?) {
        ("https", Some(c)) => endpoint.tls_config(
            ClientTlsConfig::new()
                .ca_certificate(c.ca)
                .identity(c.identity),
        )?,
        ("http", None) if local => endpoint,
        _ => bail!(
            "bird telemetry requires HTTPS and mutual TLS (plain HTTP is only allowed on numeric loopback without credentials)"
        ),
    };
    Ok(endpoint.connect_lazy())
}

struct Reporter {
    telemetry: Arc<Telemetry>,
    id: String,
    incarnation: u64,
    sequence: u64,
    client: TelemetryClient<Channel>,
}

impl Reporter {
    async fn send(&mut self, run: Option<&Arc<Run>>) -> Result<(), tonic::Status> {
        self.sequence += 1;
        let frame = run.map(|r| r.snapshot());
        let view = frame.as_ref().map(|f| &f.view);
        self.client
            .check_in(Heartbeat {
                bird_id: self.id.clone(),
                incarnation: self.incarnation,
                sequence: self.sequence,
                pid: std::process::id(),
                state: view.map_or("idle", |v| v.status.label()).into(),
                requests: view.map_or(0, |v| v.calls.count),
                failed: view.map_or(0, |v| v.calls.failed),
                sessions: view.map_or(0, |v| v.sessions),
                in_flight: view.map_or(0, |v| v.in_flight),
                p95_ms: view.and_then(|v| v.calls.p95_ms),
            })
            .await?;
        Ok(())
    }

    async fn observe(
        mut self,
        work: Option<(Arc<Run>, PathBuf)>,
        signal: impl Future<Output = Result<()>>,
    ) -> Result<()> {
        let run = work.as_ref().map(|(r, _)| r);
        self.send(run)
            .await
            .context("registering bird with the telemetry controller")?;
        self.telemetry.event(Level::Info, "bird_connected");
        println!("Bird {} connected (PID {}).", self.id, std::process::id());
        if let Some(run) = run {
            run.start(StartOptions::default())?;
        }
        // A slow telemetry acknowledgement must not delay dispatch, draining or persistence.
        let job = work
            .as_ref()
            .map(|(r, dir)| tokio::spawn(r.clone().execute(dir.clone())));
        let execute = async move {
            match job {
                Some(task) => task.await.context("bird workload task terminated")?,
                None => pending().await,
            }
        };
        tokio::pin!(execute, signal);
        let mut tick = interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut last_ack = Instant::now();
        let mut lost = false;
        let mut finished = false;
        let result = loop {
            tokio::select! {
                event = &mut signal => break event,
                result = &mut execute, if !finished => {
                    finished = true;
                    match result.and_then(|frame| crate::finish(&work.as_ref().expect("active workload").1, &frame)) {
                        Ok(()) => tick.reset_immediately(),
                        Err(err) => break Err(err),
                    }
                }
                _ = tick.tick() => match self.send(run).await {
                    Ok(()) => {
                        if lost { self.telemetry.event(Level::Info, "bird_reconnected"); }
                        lost = false;
                        last_ack = Instant::now();
                    }
                    Err(err) if matches!(err.code(), Code::Unavailable | Code::DeadlineExceeded | Code::Cancelled)
                        && last_ack.elapsed() < Duration::from_secs(30) => {
                        if !lost { self.telemetry.event(Level::Warn, "bird_contact_lost"); }
                        lost = true;
                    }
                    Err(err) => break Err(anyhow::Error::from(err).context("bird telemetry failed")),
                }
            }
        };
        let result = if !finished && let Some((run, dir)) = &work {
            run.stop();
            // Report the drained verdict to the controller even if local acceptance failed.
            execute
                .await
                .and_then(|frame| crate::finish(dir, &frame))
                .and(result)
        } else {
            result
        };
        if let Err(e) = self.send(run).await {
            self.telemetry
                .event(Level::Error, "bird_final_heartbeat_failed");
            result?;
            return Err(e).context("sending final bird telemetry");
        }
        result
    }
}

impl Bird {
    pub async fn run(
        self,
        signal: impl Future<Output = Result<()>>,
        telemetry: Arc<Telemetry>,
    ) -> Result<()> {
        let signal = async move {
            if self.supervised {
                let (send, eof) = tokio::sync::oneshot::channel();
                std::thread::Builder::new()
                    .name("supervisor-pipe".into())
                    .spawn(move || {
                        let _ =
                            send.send(std::io::Read::read(&mut std::io::stdin(), &mut [0u8; 1]));
                    })
                    .context("watching the parent process")?;
                tokio::select! {
                    res = signal => res,
                    res = eof => {
                        ensure!(res?? == 0, "unexpected supervisor input"); Ok(())
                    }
                }
            } else {
                signal.await
            }
        };
        ensure!(
            (self.report.is_none() && self.wait_ms.is_none() && self.protocol.is_none())
                || self.demo
                || self.config.is_some(),
            "--wait, --report and --protocol require a local workload (--demo or --config)"
        );
        let reporter = Reporter {
            telemetry: telemetry.clone(),
            client: TelemetryClient::new(channel(&self.home)?),
            id: self.id,
            incarnation: fastrand::u64(1..=u64::MAX),
            sequence: 0,
        };
        if self.demo {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let cfg = Config::demo(
                listener.local_addr()?,
                StartOptions {
                    users: Some(1),
                    threads: Some(1),
                    protocol: self.protocol,
                    wait_ms: self.wait_ms,
                    duration_ms: Some(15000),
                    ..Default::default()
                },
            )?;
            let job = (Run::new(cfg, telemetry), report::reserve(self.report)?);
            fixture::alongside(listener, reporter.observe(Some(job), signal)).await
        } else {
            let job = self
                .config
                .map(|path| -> Result<_> {
                    Ok((
                        Run::new(
                            Config::read(
                                &path,
                                StartOptions {
                                    endpoint: self.endpoint,
                                    protocol: self.protocol,
                                    wait_ms: self.wait_ms,
                                    ..Default::default()
                                },
                            )?,
                            telemetry,
                        ),
                        report::reserve(self.report)?,
                    ))
                })
                .transpose()?;
            reporter.observe(job, signal).await
        }
    }
}

pub fn spawn_demos(
    count: u8,
    home: &str,
    address: SocketAddr,
    report_dir: &std::path::Path,
    config: &Config,
    stop: CancellationToken,
) -> Result<JoinSet<std::io::Result<std::process::ExitStatus>>> {
    let mut children = JoinSet::new();
    if count == 0 {
        return Ok(children);
    }
    ensure!(
        address.ip().is_loopback() && home.starts_with("http://"),
        "launch demo birds only on a plaintext loopback hub; start certified birds independently for TLS"
    );
    let exe = std::env::current_exe()?;
    let wait = humantime::format_duration(config.load.wait).to_string();
    for n in 1..=count {
        let mut child = Command::new(&exe)
            .args([
                "bird",
                "--id",
                &format!("bird-{n:02}"),
                "--home",
                home,
                "--demo",
                "--protocol",
                config.target.protocol.as_str(),
                "--wait",
                &wait,
                "--supervised",
            ])
            .arg("--report")
            .arg(report_dir.join("birds").join(format!("bird-{n:02}")))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("starting a demo bird")?;
        let pipe = child.stdin.take().expect("piped bird stdin");
        let stop = stop.clone();
        children.spawn(async move {
            tokio::select! {
                res = child.wait() => res,
                _ = stop.cancelled() => {
                    drop(pipe);
                    match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
                        Ok(res) => res,
                        Err(_) => { child.kill().await?; child.wait().await }
                    }
                }
            }
        });
    }
    Ok(children)
}

pub async fn supervise(
    mut children: JoinSet<std::io::Result<std::process::ExitStatus>>,
    stop: CancellationToken,
    telemetry: Arc<Telemetry>,
) {
    loop {
        tokio::select! {
            _ = stop.cancelled() => {
                while let Some(res) = children.join_next().await {
                    if !matches!(res, Ok(Ok(s)) if s.success()) { telemetry.event(Level::Error, "demo_bird_shutdown_failed"); }
                }
                return;
            }
            Some(_) = children.join_next(), if !children.is_empty() => {
                telemetry.event(Level::Warn, "demo_bird_unexpected_exit");
            }
        }
    }
}
