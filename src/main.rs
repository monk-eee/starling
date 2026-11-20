mod config;
mod dashboard;
mod fixture;
mod mcp;
mod metrics;
mod report;
mod runner;

use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use config::{Config, DEFAULT_PATH, StartOptions, loopback_addr};
use runner::{Run, Status};

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
        #[command(flatten)]
        options: StartOptions,
        #[command(flatten)]
        output: RunOutput,
    },
    /// Serve a local MCP fixture with echo, status and reject tools.
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
    /// New report directory; defaults to a unique path under out/.
    #[arg(long)]
    report: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Init { file } => {
            config::write_starter(&file)?;
            println!(
                "Created {}. Set the target and tools, or use `starling fixture` to try the defaults.",
                file.display()
            );
        }
        Command::Demo { options, output } => {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .context("binding demo fixture")?;
            let address = listener.local_addr()?;
            let cfg = Config::demo(address, options)?;
            println!("Demo fixture: http://{address}/mcp");
            let done = CancellationToken::new();
            let server = fixture::serve(listener, done.clone());
            tokio::pin!(server);
            tokio::select! {
                result = &mut server => { result?; bail!("demo fixture stopped unexpectedly"); }
                result = run_workload(cfg, output, 0) => {
                    done.cancel();
                    server.await?;
                    result?;
                }
            }
        }
        Command::Validate { config, options } => {
            let cfg = Config::read(&config, options)?;
            println!(
                "Valid: {} users, {} workload threads, {} weighted tools; duration {}, wait {}, timeout {}",
                cfg.load.users,
                cfg.load.threads,
                cfg.tasks.len(),
                humantime::format_duration(cfg.run.duration),
                humantime::format_duration(cfg.load.wait),
                humantime::format_duration(cfg.load.timeout)
            );
        }
        Command::Fixture { listen } => {
            let listener = TcpListener::bind(listen).await.context("binding fixture")?;
            println!("Fixture: http://{}/mcp", listener.local_addr()?);
            let done = CancellationToken::new();
            let server = fixture::serve(listener, done.clone());
            tokio::pin!(server);
            tokio::select! {
                result = &mut server => result?,
                signal = tokio::signal::ctrl_c() => { signal?; done.cancel(); server.await?; }
            }
        }
        Command::Run {
            config,
            options,
            output,
        } => {
            run_workload(Config::read(&config, options)?, output, 8787).await?;
        }
    }
    Ok(())
}

async fn run_workload(cfg: Config, output: RunOutput, dashboard_port: u16) -> Result<()> {
    let run = Run::new(cfg);
    let listener = if output.web {
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
        println!("Dashboard: http://{}", listener.local_addr()?);
        dashboard::serve(listener, run, dir).await?
    } else {
        run.start(Default::default())?;
        let execute = run.clone().execute(dir.clone());
        tokio::pin!(execute);
        let frame = tokio::select! {
            result = &mut execute => result?,
            signal = tokio::signal::ctrl_c() => { signal?; run.stop(); execute.await? }
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
