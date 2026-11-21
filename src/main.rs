mod bird;
mod config;
mod dashboard;
mod fixture;
mod flock;
mod mcp;
mod metrics;
mod report;
mod runner;
mod telemetry;

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use config::{Config, DEFAULT_PATH, StartOptions, loopback_addr};
use runner::{Run, Status};
use telemetry::{Level, Logging, Telemetry};

#[derive(Parser)]
#[command(
    version,
    about,
    after_help = "Try it: starling demo --duration 5s\nCreate a workload: starling init"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a starter workload in a new YAML or TOML file.
    Init {
        #[arg(value_name = "FILE", default_value = DEFAULT_PATH)]
        file: PathBuf,
    },
    /// Try Starling with its own local fixture; no config or second terminal needed.
    Demo {
        /// Launch 1-8 independent self-check birds, each with its own local fixture.
        #[arg(long, requires = "web", value_parser = clap::value_parser!(u8).range(1..=8))]
        birds: Option<u8>,
        #[command(flatten)]
        options: StartOptions,
        #[command(flatten)]
        output: RunOutput,
    },
    /// Phone home with one process's live telemetry. The controller cannot assign work.
    Bird(bird::Bird),
    /// Serve a local MCP fixture with echo, status and reject tools and an ask prompt.
    Fixture {
        #[arg(long, default_value = "127.0.0.1:8081", value_parser = loopback_addr)]
        listen: SocketAddr,
    },
    /// Check the workload and local limits, without sending requests.
    Validate {
        /// TOML or YAML workload file.
        #[arg(long, default_value = DEFAULT_PATH)]
        config: PathBuf,
        #[command(flatten)]
        options: StartOptions,
    },
    /// Run a workload and persist a new report bundle.
    Run {
        /// TOML or YAML workload file.
        #[arg(long, default_value = DEFAULT_PATH)]
        config: PathBuf,
        #[command(flatten)]
        options: StartOptions,
        #[command(flatten)]
        output: RunOutput,
    },
}

#[derive(Args)]
struct RunOutput {
    /// Start the workload from the local browser dashboard.
    #[arg(long, conflicts_with = "headless")]
    web: bool,
    /// Start immediately without the dashboard (default).
    #[arg(long)]
    headless: bool,
    /// Dashboard address. Port 0 selects a free port.
    #[arg(long, value_parser = loopback_addr, requires = "web")]
    listen: Option<SocketAddr>,
    /// Separate gRPC telemetry listener (default: a free loopback port). Non-loopback requires mTLS.
    #[arg(long, requires = "web", value_name = "IP:PORT")]
    bird_listen: Option<SocketAddr>,
    /// New report directory; defaults to a unique path under out/.
    #[arg(long)]
    report: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = Cli::parse().command;
    // The collector client and the mTLS flock must share the same TLS provider.
    #[cfg(feature = "otlp")]
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("could not select the Starling TLS provider"))?;
    let (role, bird) = match &command {
        Command::Bird(args) => ("bird", Some(args.id.as_str())),
        Command::Demo { output, .. } | Command::Run { output, .. } => {
            (if output.web { "controller" } else { "runner" }, None)
        }
        Command::Fixture { .. } => ("fixture", None),
        Command::Init { .. } | Command::Validate { .. } => ("cli", None),
    };
    let logging = Logging::open(role, bird, role != "cli").await?;
    logging.events.event(Level::Info, "process_started");
    let result = execute(command, logging.events.clone()).await;
    if result.is_err() {
        logging.events.event(Level::Error, "process_failed");
    } else {
        logging.events.event(Level::Info, "process_finished");
    }
    match (result, logging.shutdown().await) {
        (Err(command), Err(flush)) => Err(command.context(format!("telemetry: {flush}"))),
        (result, flushed) => result.and(flushed),
    }
}

async fn execute(command: Command, telemetry: Arc<Telemetry>) -> Result<()> {
    match command {
        Command::Init { file } => {
            config::write_starter(&file)?;
            println!(
                "Created {}. Set the target and tools, or use `starling fixture` to try the defaults.",
                file.display()
            );
        }
        Command::Bird(args) => args.run(shutdown_signal()?, telemetry).await?,
        Command::Demo {
            options,
            output,
            birds,
        } => {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .context("binding demo fixture")?;
            let address = listener.local_addr()?;
            let cfg = Config::demo(address, options)?;
            let signal = shutdown_signal()?;
            println!("Demo fixture: http://{address}/mcp");
            fixture::alongside(
                listener,
                run_workload(
                    Run::new(cfg, telemetry),
                    output,
                    Some(address),
                    birds.unwrap_or(0),
                    signal,
                ),
            )
            .await?;
        }
        Command::Validate { config, options } => {
            let cfg = Config::read(&config, options)?;
            println!(
                "Valid: {} users, {} workload threads, {} weighted requests; duration {}, wait {}, timeout {}; endpoint {} ({})",
                cfg.load.users,
                cfg.load.threads,
                cfg.tasks.len(),
                humantime::format_duration(cfg.run.duration),
                humantime::format_duration(cfg.load.wait),
                humantime::format_duration(cfg.load.timeout),
                cfg.target.selected.as_deref().unwrap_or("default"),
                cfg.target.endpoint().origin().ascii_serialization(),
            );
        }
        Command::Fixture { listen } => {
            let listener = TcpListener::bind(listen).await.context("binding fixture")?;
            let signal = shutdown_signal()?;
            println!("Fixture: http://{}/mcp", listener.local_addr()?);
            let done = CancellationToken::new();
            let server = fixture::serve(listener, done.clone());
            tokio::pin!(server);
            tokio::select! {
                result = &mut server => result?,
                event = signal => { event?; done.cancel(); server.await?; }
            }
        }
        Command::Run {
            config,
            options,
            output,
        } => {
            run_workload(
                Run::new(Config::read(&config, options)?, telemetry),
                output,
                None,
                0,
                shutdown_signal()?,
            )
            .await?;
        }
    }
    Ok(())
}

fn shutdown_signal() -> Result<impl Future<Output = Result<()>>> {
    // Install Unix handlers before advertising readiness, including when running as PID 1.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut interrupt = signal(SignalKind::interrupt()).context("listening for SIGINT")?;
        let mut terminate = signal(SignalKind::terminate()).context("listening for SIGTERM")?;
        Ok(async move {
            tokio::select! {
                event = interrupt.recv() => event.context("SIGINT listener closed"),
                event = terminate.recv() => event.context("SIGTERM listener closed"),
            }
        })
    }
    #[cfg(not(unix))]
    {
        Ok(async {
            tokio::signal::ctrl_c()
                .await
                .context("listening for Ctrl+C")
        })
    }
}

async fn run_workload(
    run: Arc<Run>,
    output: RunOutput,
    demo_fixture: Option<SocketAddr>,
    birds: u8,
    signal: impl Future<Output = Result<()>>,
) -> Result<()> {
    let listener = if output.web {
        let dashboard_port = if demo_fixture.is_some() { 0 } else { 8787 };
        Some(
            TcpListener::bind(
                output
                    .listen
                    .unwrap_or(([127, 0, 0, 1], dashboard_port).into()),
            )
            .await
            .context("binding dashboard")?,
        )
    } else {
        None
    };
    let dir = report::reserve(output.report)?;
    let (dir, frame) = if let Some(listener) = listener {
        dashboard::serve(
            listener,
            run,
            dir,
            demo_fixture,
            output.bird_listen.unwrap_or(([127, 0, 0, 1], 0).into()),
            birds,
            signal,
        )
        .await?
    } else {
        run.start(Default::default())?;
        let execute = run.clone().execute(dir.clone());
        tokio::pin!(execute);
        let frame = tokio::select! {
            result = &mut execute => result?,
            event = signal => { event?; run.stop(); execute.await? }
        };
        (dir, frame)
    };
    finish(&dir, &frame)
}

fn finish(dir: &std::path::Path, frame: &runner::Frame) -> Result<()> {
    println!(
        "{} calls, {} failed. Report: {}",
        frame.view.calls.count,
        frame.view.calls.failed,
        dir.join("index.html").display()
    );
    if frame.view.status == Status::Failed {
        bail!("{}", frame.view.reason.as_deref().unwrap_or("run failed"));
    }
    Ok(())
}
