use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use clap::Args;
use serde_json::json;
use tokio::{process::Command, sync::watch, task::JoinSet};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use url::Url;

use crate::{
    bird,
    config::Protocol,
    conformance::{SUITE, Selection, Verdict, WorkerReport},
    flock::{
        self, Hub,
        rpc::{
            CheckSubmission, Receipt, check_collector_client::CheckCollectorClient,
            check_collector_server::CheckCollector,
        },
    },
    report,
};

#[cfg(test)]
mod tests;

pub async fn submit(home: &Url, campaign: String, result: &WorkerReport) -> Result<()> {
    let mut client = CheckCollectorClient::new(bird::channel(home)?);
    client
        .submit(CheckSubmission {
            campaign,
            report_json: serde_json::to_vec(result)?,
        })
        .await
        .context("submitting conformance report")?;
    Ok(())
}

#[derive(Clone)]
pub struct Inbox(Arc<Plan>);
struct Plan {
    campaign: String,
    expected: BTreeSet<String>,
    revisions: Vec<Protocol>,
    deadline: Instant,
    accepting: Mutex<bool>,
    reports: watch::Sender<BTreeMap<String, WorkerReport>>,
}

#[tonic::async_trait]
impl CheckCollector for Inbox {
    async fn submit(&self, req: Request<CheckSubmission>) -> Result<Response<Receipt>, Status> {
        let submission = req.into_inner();
        if submission.campaign != self.0.campaign {
            return Err(Status::failed_precondition("different campaign"));
        }
        let report: WorkerReport = serde_json::from_slice(&submission.report_json)
            .map_err(|_| Status::invalid_argument("invalid conformance report"))?;
        if !self.0.expected.contains(&report.bird_id) {
            return Err(Status::permission_denied("unexpected worker"));
        }
        if !report.validate_for(&self.0.revisions) {
            return Err(Status::invalid_argument(
                "incomplete or incompatible check evidence",
            ));
        }
        let accepting = self.0.accepting.lock().expect("check inbox lock");
        if !*accepting || Instant::now() >= self.0.deadline {
            return Err(Status::failed_precondition("campaign has ended"));
        }
        let mut conflict = false;
        self.0.reports.send_if_modified(|reports| {
            if let Some(previous) = reports.get(&report.bird_id) {
                conflict = previous != &report;
                return false;
            }
            reports.insert(report.bird_id.clone(), report);
            true
        });
        if conflict {
            return Err(Status::already_exists(
                "different report already submitted for this worker",
            ));
        }
        Ok(Response::new(Receipt {}))
    }
}

#[derive(Args)]
pub struct Collect {
    /// Comma-separated public IDs of the workers that must submit evidence.
    #[arg(long, value_delimiter=',', value_parser=flock::bird_id, required_unless_present="demo_birds", conflicts_with="demo_birds")]
    expect: Vec<String>,
    /// Launch this many independent local worker processes, each with its own fixture.
    #[arg(long, value_name="N", value_parser=clap::value_parser!(u8).range(1..=8))]
    demo_birds: Option<u8>,
    #[arg(long, default_value = "127.0.0.1:0")]
    listen: SocketAddr,
    /// Deadline to receive all expected reports.
    #[arg(long, default_value_t=60, value_parser=clap::value_parser!(u64).range(1..=900))]
    deadline_secs: u64,
    #[command(flatten)]
    selection: Selection,
    #[arg(long)]
    report: Option<PathBuf>,
}

impl Collect {
    pub async fn run(self, signal: impl Future<Output = Result<()>>) -> Result<()> {
        let revisions = self.selection.resolve(&Protocol::all())?;
        ensure!(
            !revisions.contains(&Protocol::Auto),
            "collect requires exact revisions so missing coverage is detectable"
        );
        let names = if let Some(n) = self.demo_birds {
            (1..=n).map(|i| format!("bird-{i:02}")).collect()
        } else {
            self.expect
        };
        let expected: BTreeSet<_> = names.iter().cloned().collect();
        ensure!(
            !expected.is_empty() && expected.len() <= 64 && expected.len() == names.len(),
            "expect 1-64 distinct worker IDs"
        );
        let hub = Hub::bind(self.listen).await?;
        let home = hub.endpoint.clone();
        ensure!(
            self.demo_birds.is_none()
                || (self.listen.ip().is_loopback() && home.starts_with("http://")),
            "launch demo workers only on plaintext loopback; start certified workers independently for TLS"
        );
        let dir = report::reserve(self.report)?;
        let campaign = format!("{:032x}", fastrand::u128(..));
        let (reports, mut feed) = watch::channel(BTreeMap::new());
        let deadline = Instant::now() + Duration::from_secs(self.deadline_secs);
        let plan = Arc::new(Plan {
            campaign,
            expected,
            revisions,
            reports,
            deadline,
            accepting: Mutex::new(true),
        });
        let stop = CancellationToken::new();
        let _guard = stop.clone().drop_guard();
        let mut server =
            tokio::spawn(hub.serve_with_checks(stop.clone(), Some(Inbox(plan.clone()))));
        println!("Check collector: {home}");
        println!("Check campaign: {}", plan.campaign);
        let mut children = JoinSet::new();
        if self.demo_birds.is_some() {
            let exe = std::env::current_exe()?;
            let versions = plan
                .revisions
                .iter()
                .map(|r| r.as_str())
                .collect::<Vec<_>>()
                .join(",");
            for id in &plan.expected {
                let mut child = Command::new(&exe)
                    .args([
                        "check",
                        "--demo",
                        "--protocol",
                        &versions,
                        "--id",
                        id,
                        "--home",
                        &home,
                        "--campaign",
                        &plan.campaign,
                        "--report",
                    ])
                    .arg(dir.join("birds").join(id))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .context("starting conformance worker")?;
                let id = id.clone();
                children.spawn(async move { (id, child.wait().await) });
            }
        }
        let deadline = tokio::time::sleep_until(deadline.into());
        tokio::pin!(deadline, signal);
        let mut worker_errors = Vec::new();
        let finish_reason = loop {
            if children.is_empty()
                && (feed.borrow().len() == plan.expected.len() || self.demo_birds.is_some())
            {
                break if feed.borrow().len() == plan.expected.len() {
                    "complete"
                } else {
                    "workers_exited"
                };
            }
            tokio::select! {
                res=&mut server => {res??;anyhow::bail!("check collector stopped unexpectedly");}
                _=&mut deadline => break "deadline",
                res=&mut signal => {res?;break "interrupted";}
                changed=feed.changed()=>{changed?;}
                Some(res)=children.join_next(), if !children.is_empty()=>{
                    let (id,exit)=res?;
                    if !exit?.success() {worker_errors.push(id);}
                }
            }
        };
        *plan.accepting.lock().expect("check inbox lock") = false;
        let received = feed.borrow().clone();
        children.shutdown().await;
        stop.cancel();
        server.await??;
        let missing: Vec<_> = plan
            .expected
            .iter()
            .filter(|id| !received.contains_key(*id))
            .collect();
        let mut verdict = Verdict::aggregate(received.values().map(|r| r.verdict));
        if verdict == Verdict::Pass
            && (finish_reason != "complete" || !missing.is_empty() || !worker_errors.is_empty())
        {
            verdict = Verdict::Inconclusive;
        }
        report::json(
            &dir,
            "fleet-conformance.json",
            &json!({
                "schema_version":1,"suite":SUITE,"campaign":plan.campaign,"verdict":verdict,
                "expected_birds":plan.expected,"expected_revisions":plan.revisions,"missing_birds":missing,"finish_reason":finish_reason,
                "worker_errors":worker_errors,"workers":received,
            }),
        )?;
        println!(
            "Fleet conformance: {} ({}/{} workers). Report: {}",
            verdict.label(),
            received.len(),
            plan.expected.len(),
            dir.join("fleet-conformance.json").display()
        );
        ensure!(
            verdict == Verdict::Pass,
            "fleet conformance: {}",
            verdict.label()
        );
        Ok(())
    }
}
