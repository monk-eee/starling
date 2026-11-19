mod config;
mod dashboard;
mod fixture;
mod mcp;
mod metrics;
mod report;
mod runner;

use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use config::{Config, StartOptions, loopback_addr};
use runner::{Run, Status};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve a local MCP fixture with echo, status and reject tools.
    Fixture {
        #[arg(long, default_value = "127.0.0.1:8081", value_parser = loopback_addr)]
        listen: SocketAddr,
    },
    /// Check the workload and local limits, without sending requests.
    Validate {
        /// TOML or YAML workload file.
        #[arg(long)]
        config: PathBuf,
        #[command(flatten)]
        options: StartOptions,
    },
    /// Run a workload and persist a new report bundle.
    Run {
        /// TOML or YAML workload file.
        #[arg(long)]
        config: PathBuf,
        #[command(flatten)]
        options: StartOptions,
        /// Open a local control plane; start the workload from the dashboard.
        #[arg(long, conflicts_with = "headless")]
        web: bool,
        /// Start immediately without opening a listening socket (default).
        #[arg(long)]
        headless: bool,
        /// Dashboard address. Port 0 selects a free port.
        #[arg(long, value_parser = loopback_addr, requires = "web")]
        listen: Option<SocketAddr>,
        /// New directory for this run; defaults to a unique path under out/.
        #[arg(long)]
        report: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
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
            web,
            listen,
            report,
            ..
        } => {
            let run = Run::new(Config::read(&config, options)?);
            let listener = if web {
                Some(
                    TcpListener::bind(listen.unwrap_or(([127, 0, 0, 1], 8787).into()))
                        .await
                        .context("binding dashboard")?,
                )
            } else {
                None
            };
            let dir = report::reserve(report)?;
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
            finish(&dir, &frame)?;
        }
    }
    Ok(())
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
