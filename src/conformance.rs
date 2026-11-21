use std::{path::PathBuf, time::Duration};

use anyhow::{Result, ensure};
use clap::Args;
use rmcp::{
    ServiceError,
    model::{ClientRequest, ErrorCode, ResultType, ServerResult},
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, time::timeout};
use url::Url;

use crate::{
    config::{Config, DEFAULT_PATH, Protocol, StartOptions, Target},
    fixture,
    flock::bird_id,
    mcp::Session,
    metrics::Outcome,
    report,
};

const SCHEMA: u32 = 1;
pub const SUITE: &str = "mcp.streamable-http.core.v1";

#[derive(Clone, Default, Args, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    /// One revision, or a comma-separated matrix of exact revisions.
    #[arg(long, value_delimiter = ',', conflicts_with = "all_protocols")]
    pub protocol: Vec<Protocol>,
    /// Check every supported Streamable HTTP revision independently.
    #[arg(long)]
    pub all_protocols: bool,
}

impl Selection {
    pub fn for_config(&self, cfg: &Config) -> Result<Vec<Protocol>> {
        if cfg.conformance.revisions.is_empty() {
            self.resolve(&[cfg.target.protocol])
        } else {
            self.resolve(&cfg.conformance.revisions)
        }
    }

    pub fn resolve(&self, defaults: &[Protocol]) -> Result<Vec<Protocol>> {
        ensure!(
            !(self.all_protocols && !self.protocol.is_empty()),
            "choose a revision list or all protocols, not both"
        );
        let revisions = if self.all_protocols {
            Protocol::all()
        } else if self.protocol.is_empty() {
            defaults.to_vec()
        } else {
            self.protocol.clone()
        };
        ensure!(!revisions.is_empty(), "at least one revision is required");
        Protocol::validate_matrix(&revisions)?;
        Ok(revisions)
    }
}

#[derive(Args)]
pub struct Check {
    /// Check an ephemeral built-in fixture; no workload file needed.
    #[arg(long, conflicts_with = "config")]
    demo: bool,
    /// Use the target and operation timeout from this local workload.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Check a local Streamable HTTP MCP endpoint directly, without a workload file.
    #[arg(long, conflicts_with_all = ["config", "demo"])]
    url: Option<Url>,
    #[arg(long, requires = "config")]
    endpoint: Option<String>,
    #[command(flatten)]
    selection: Selection,
    /// Public worker ID (required when submitting to a collector).
    #[arg(long, value_parser = bird_id)]
    pub id: Option<String>,
    /// Conformance collector origin; remote collectors require mutual TLS.
    #[arg(long, requires_all = ["campaign", "id"])]
    home: Option<Url>,
    /// Campaign ID printed by that collector.
    #[arg(long, requires = "home", value_parser = bird_id)]
    campaign: Option<String>,
    /// New directory for the machine-readable conformance report.
    #[arg(long)]
    report: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    Unsupported,
    Inconclusive,
    Skip,
    NotRun,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unsupported => "unsupported",
            Self::Inconclusive => "inconclusive",
            Self::Skip => "skip",
            Self::NotRun => "not_run",
        }
    }

    pub fn aggregate(items: impl Iterator<Item = Self>) -> Self {
        let mut result = Self::NotRun;
        for item in items {
            match item {
                Self::Fail => return Self::Fail,
                Self::Unsupported => result = Self::Unsupported,
                Self::Inconclusive if result != Self::Unsupported => result = Self::Inconclusive,
                Self::Pass if result == Self::NotRun => result = Self::Pass,
                _ => {}
            }
        }
        if result == Self::NotRun {
            Self::Inconclusive
        } else {
            result
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Rule {
    #[serde(rename = "lifecycle.revision")]
    Lifecycle,
    #[serde(rename = "catalog.tools")]
    Tools,
    #[serde(rename = "catalog.prompts")]
    Prompts,
    #[serde(rename = "catalog.resources")]
    Resources,
    #[serde(rename = "transport.close")]
    Close,
}
const RULES: [Rule; 5] = [
    Rule::Lifecycle,
    Rule::Tools,
    Rule::Prompts,
    Rule::Resources,
    Rule::Close,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Evidence {
    Initialized,
    Discovered,
    AutoNegotiated,
    CatalogPage,
    CatalogPageWithCacheHints,
    Closed,
    NotAdvertised,
    StartupNotEstablished,
    RevisionUnsupported,
    WrongResponse,
    InvalidToolSchema,
    InvalidResourceUri,
    MissingCompletionFields,
    MethodMissing,
    TransportError,
    Timeout,
    RpcError,
    InputRequired,
    CleanupError,
}

impl Evidence {
    fn status(self) -> Verdict {
        use Evidence::*;
        match self {
            Initialized
            | Discovered
            | AutoNegotiated
            | CatalogPage
            | CatalogPageWithCacheHints
            | Closed => Verdict::Pass,
            NotAdvertised => Verdict::Skip,
            StartupNotEstablished => Verdict::NotRun,
            RevisionUnsupported => Verdict::Unsupported,
            WrongResponse
            | InvalidToolSchema
            | InvalidResourceUri
            | MissingCompletionFields
            | MethodMissing => Verdict::Fail,
            TransportError | Timeout | RpcError | InputRequired | CleanupError => {
                Verdict::Inconclusive
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    rule: Rule,
    status: Verdict,
    evidence: Evidence,
}

impl Finding {
    fn new(rule: Rule, evidence: Evidence) -> Self {
        Self {
            rule,
            status: evidence.status(),
            evidence,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileReport {
    pub requested: Protocol,
    negotiated: Option<Protocol>,
    specification: Option<String>,
    verdict: Verdict,
    checks: Vec<Finding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReport {
    schema_version: u32,
    suite: String,
    pub bird_id: String,
    pub pid: u32,
    pub verdict: Verdict,
    pub profiles: Vec<ProfileReport>,
}

impl WorkerReport {
    pub fn validate_for(&self, expected: &[Protocol]) -> bool {
        self.schema_version == SCHEMA
            && self.suite == SUITE
            && self.pid != 0
            && bird_id(&self.bird_id).is_ok()
            && self.profiles.len() == expected.len()
            && expected
                .iter()
                .all(|r| self.profiles.iter().filter(|p| p.requested == *r).count() == 1)
            && self.verdict == Verdict::aggregate(self.profiles.iter().map(|p| p.verdict))
            && self.profiles.iter().all(|p| {
                if p.specification != spec_link(p.requested, p.negotiated)
                    || !p.checks.iter().map(|r| r.rule).eq(RULES)
                    || p.verdict != Verdict::aggregate(p.checks.iter().map(|r| r.status))
                    || !p.checks.iter().all(|r| r.status == r.evidence.status())
                {
                    return false;
                }
                let start = p.checks[0].evidence;
                let Some(Protocol::Exact(actual)) = p.negotiated else {
                    return p.negotiated.is_none()
                        && matches!(
                            start,
                            Evidence::RevisionUnsupported
                                | Evidence::WrongResponse
                                | Evidence::TransportError
                                | Evidence::Timeout
                                | Evidence::RpcError
                        )
                        && p.checks[1..]
                            .iter()
                            .all(|r| r.evidence == Evidence::StartupNotEstablished);
                };
                p.requested.accepts(&actual.revision)
                    && start
                        == if p.requested == Protocol::Auto {
                            Evidence::AutoNegotiated
                        } else if actual.revision.has_initialize() {
                            Evidence::Initialized
                        } else {
                            Evidence::Discovered
                        }
                    && p.checks.iter().skip(1).all(|r| match r.rule {
                        Rule::Close => {
                            matches!(r.evidence, Evidence::Closed | Evidence::CleanupError)
                        }
                        Rule::Lifecycle => false,
                        _ => match r.evidence {
                            Evidence::CatalogPage => !actual.require_catalog_hints,
                            Evidence::CatalogPageWithCacheHints
                            | Evidence::MissingCompletionFields => actual.require_catalog_hints,
                            Evidence::InvalidToolSchema => r.rule == Rule::Tools,
                            Evidence::InvalidResourceUri => r.rule == Rule::Resources,
                            Evidence::NotAdvertised
                            | Evidence::WrongResponse
                            | Evidence::MethodMissing
                            | Evidence::RpcError
                            | Evidence::TransportError
                            | Evidence::Timeout
                            | Evidence::InputRequired => true,
                            _ => false,
                        },
                    })
            })
    }
}

fn spec_link(requested: Protocol, negotiated: Option<Protocol>) -> Option<String> {
    match negotiated.unwrap_or(requested) {
        Protocol::Exact(v) => Some(format!(
            "https://modelcontextprotocol.io/specification/{}",
            v.revision
        )),
        Protocol::Auto => None,
    }
}

async fn catalog(session: &Session, rule: Rule, require_hints: bool) -> Evidence {
    let peer = session.peer();
    let caps = &peer.peer_info().expect("established session").capabilities;
    let request = match rule {
        Rule::Tools if caps.tools.is_some() => ClientRequest::ListToolsRequest(Default::default()),
        Rule::Prompts if caps.prompts.is_some() => {
            ClientRequest::ListPromptsRequest(Default::default())
        }
        Rule::Resources if caps.resources.is_some() => {
            ClientRequest::ListResourcesRequest(Default::default())
        }
        Rule::Tools | Rule::Prompts | Rule::Resources => return Evidence::NotAdvertised,
        Rule::Lifecycle | Rule::Close => unreachable!("not a catalog rule"),
    };
    let (kind, ttl, scope) = match (rule, peer.send_request(request).await) {
        (Rule::Tools, Ok(ServerResult::ListToolsResult(page))) => {
            if page
                .tools
                .iter()
                .any(|t| t.input_schema.get("type").and_then(|v| v.as_str()) != Some("object"))
            {
                return Evidence::InvalidToolSchema;
            }
            (page.result_type, page.ttl_ms, page.cache_scope)
        }
        (Rule::Prompts, Ok(ServerResult::ListPromptsResult(p))) => {
            (p.result_type, p.ttl_ms, p.cache_scope)
        }
        (Rule::Resources, Ok(ServerResult::ListResourcesResult(p))) => {
            if p.resources.iter().any(|r| Url::parse(&r.uri).is_err()) {
                return Evidence::InvalidResourceUri;
            }
            (p.result_type, p.ttl_ms, p.cache_scope)
        }
        (_, Ok(ServerResult::InputRequiredResult(_))) => return Evidence::InputRequired,
        (_, Ok(_) | Err(ServiceError::UnexpectedResponse)) => return Evidence::WrongResponse,
        (_, Err(ServiceError::McpError(e))) if e.code == ErrorCode::METHOD_NOT_FOUND => {
            return Evidence::MethodMissing;
        }
        (_, Err(ServiceError::McpError(_))) => return Evidence::RpcError,
        (_, Err(ServiceError::Timeout { .. })) => return Evidence::Timeout,
        (_, Err(_)) => return Evidence::TransportError,
    };
    if !require_hints {
        Evidence::CatalogPage
    } else if kind == Some(ResultType::COMPLETE) && ttl.is_some() && scope.is_some() {
        Evidence::CatalogPageWithCacheHints
    } else {
        Evidence::MissingCompletionFields
    }
}

async fn profile(mut target: Target, requested: Protocol, limit: Duration) -> ProfileReport {
    target.protocol = requested;
    let mut negotiated = None;
    let mut checks = Vec::new();
    let opened = timeout(limit, Session::open(&target, &[], limit))
        .await
        .unwrap_or(Err(Outcome::Timeout));
    match opened {
        Ok(s) => {
            let actual: Protocol = s
                .version()
                .parse()
                .expect("accepted protocol is registered");
            negotiated = Some(actual);
            let Protocol::Exact(standard) = actual else {
                unreachable!("negotiated revision cannot be auto")
            };
            checks.push(Finding::new(
                Rule::Lifecycle,
                if requested == Protocol::Auto {
                    Evidence::AutoNegotiated
                } else if standard.revision.has_initialize() {
                    Evidence::Initialized
                } else {
                    Evidence::Discovered
                },
            ));
            for rule in RULES
                .into_iter()
                .filter(|r| !matches!(r, Rule::Lifecycle | Rule::Close))
            {
                checks.push(Finding::new(
                    rule,
                    timeout(limit, catalog(&s, rule, standard.require_catalog_hints))
                        .await
                        .unwrap_or(Evidence::Timeout),
                ));
            }
            let closed = s.close(limit).await;
            checks.push(Finding::new(
                Rule::Close,
                if closed.is_ok() {
                    Evidence::Closed
                } else {
                    Evidence::CleanupError
                },
            ));
        }
        Err(e) => {
            let evidence = match e {
                Outcome::ProtocolVersion => Evidence::RevisionUnsupported,
                Outcome::RpcError => Evidence::RpcError,
                Outcome::Timeout => Evidence::Timeout,
                Outcome::Protocol => Evidence::WrongResponse,
                _ => Evidence::TransportError,
            };
            checks.extend(RULES.map(|r| {
                Finding::new(
                    r,
                    if r == Rule::Lifecycle {
                        evidence
                    } else {
                        Evidence::StartupNotEstablished
                    },
                )
            }));
        }
    }
    ProfileReport {
        requested,
        negotiated,
        specification: spec_link(requested, negotiated),
        verdict: Verdict::aggregate(checks.iter().map(|r| r.status)),
        checks,
    }
}

pub async fn inspect(cfg: &Config, revisions: &[Protocol], bird: &str) -> WorkerReport {
    let mut profiles = Vec::with_capacity(revisions.len());
    for rev in revisions {
        profiles.push(profile(cfg.target.clone(), *rev, cfg.load.timeout).await);
    }
    WorkerReport {
        schema_version: SCHEMA,
        suite: SUITE.into(),
        bird_id: bird.into(),
        pid: std::process::id(),
        verdict: Verdict::aggregate(profiles.iter().map(|p| p.verdict)),
        profiles,
    }
}

impl Check {
    pub async fn run(self) -> Result<()> {
        let bird = self.id.as_deref().unwrap_or("local");
        let options = StartOptions {
            endpoint: self.endpoint,
            ..Default::default()
        };
        let (cfg, listener) = if self.demo {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            (
                Config::demo(listener.local_addr()?, options)?,
                Some(listener),
            )
        } else if let Some(url) = self.url {
            (Config::check_target(url)?, None)
        } else {
            (
                Config::read(
                    self.config
                        .as_deref()
                        .unwrap_or(std::path::Path::new(DEFAULT_PATH)),
                    options,
                )?,
                None,
            )
        };
        let revisions = self.selection.for_config(&cfg)?;
        let dir = report::reserve(self.report)?;
        let probe = inspect(&cfg, &revisions, bird);
        let result = if let Some(listener) = listener {
            fixture::alongside(listener, async { Ok(probe.await) }).await?
        } else {
            probe.await
        };
        report::json(&dir, "conformance.json", &result)?;
        for p in &result.profiles {
            println!(
                "MCP {}: {} ({}/{} checks passed)",
                p.requested.as_str(),
                p.verdict.label(),
                p.checks
                    .iter()
                    .filter(|r| r.status == Verdict::Pass)
                    .count(),
                p.checks.len()
            );
            for finding in p.checks.iter().filter(|r| {
                matches!(
                    r.status,
                    Verdict::Fail | Verdict::Unsupported | Verdict::Inconclusive
                )
            }) {
                println!("  {}", serde_json::to_string(finding)?);
            }
        }
        println!(
            "Conformance: {}. Report: {}",
            result.verdict.label(),
            dir.join("conformance.json").display()
        );
        if let (Some(home), Some(campaign)) = (self.home, self.campaign) {
            crate::check_fleet::submit(&home, campaign, &result).await?;
        }
        ensure!(
            result.verdict == Verdict::Pass,
            "conformance profile: {}",
            result.verdict.label()
        );
        Ok(())
    }
}
