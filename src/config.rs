use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use rmcp::model::ProtocolVersion;
use serde::Deserialize;
use serde_json::{Map, Value};
use url::Url;

pub const MAX_DURATION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_OPERATION_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_PATH: &str = "starling.yaml";
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
pub const STARTER_YAML: &str = include_str!("../examples/local.yaml");
const STARTER_TOML: &str = include_str!("../examples/local.toml");

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"))
}

pub fn write_starter(path: &Path) -> Result<()> {
    let template = if is_yaml(path) {
        STARTER_YAML
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
    {
        STARTER_TOML
    } else {
        bail!("choose a starter filename ending in .yaml, .yml, or .toml");
    };
    File::create_new(path)
        .with_context(|| format!("creating {} (a new file is required)", path.display()))?
        .write_all(template.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub run: RunConfig,
    pub target: Target,
    pub load: Load,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub report: crate::report::ReportOptions,
    #[serde(default)]
    pub conformance: ConformanceOptions,
    #[serde(
        default,
        rename = "assert",
        deserialize_with = "crate::acceptance::Criteria::from_map"
    )]
    pub acceptance: Option<crate::acceptance::Criteria>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConformanceOptions {
    pub revisions: Vec<Protocol>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub name: String,
    #[serde(default)]
    pub mode: RunMode,
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    pub seed: u64,
}

#[derive(Clone, Copy, Default, PartialEq, serde::Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    #[default]
    Weighted,
    Once,
}

impl RunMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Weighted => "weighted",
            Self::Once => "once-per-case",
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub url: Url,
    #[serde(default)]
    pub endpoints: BTreeMap<String, Url>,
    #[serde(default, rename = "select")]
    pub selected: Option<String>,
    #[serde(default)]
    pub protocol: Protocol,
}

impl Target {
    pub fn endpoint(&self) -> &Url {
        match &self.selected {
            Some(name) => &self.endpoints[name],
            None => &self.url,
        }
    }
}

fn validate_url(url: &mut Url) -> Result<()> {
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "target must be an HTTP or HTTPS URL with a host and without credentials, query or fragment"
    );
    // Preserve the local HTTP fixture convention, but never rewrite a TLS certificate name.
    if url.scheme() == "http" && url.host_str() == Some("localhost") {
        url.set_host(Some("127.0.0.1"))?;
    }
    Ok(())
}

fn valid_label(s: &str) -> bool {
    s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Protocol {
    #[default]
    Auto,
    Exact(&'static Standard),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Standard {
    pub revision: ProtocolVersion,
    pub require_catalog_hints: bool,
}

impl Standard {
    const fn new(revision: ProtocolVersion, require_catalog_hints: bool) -> Self {
        Self {
            revision,
            require_catalog_hints,
        }
    }
}

impl serde::Serialize for Standard {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.revision.as_str())
    }
}

impl serde::Serialize for Protocol {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl Protocol {
    // Streamable HTTP starts at 2025-03-26; the older HTTP+SSE transport is not interchangeable.
    pub const REVISIONS: &'static [Standard] = &[
        Standard::new(ProtocolVersion::V_2026_07_28, true),
        Standard::new(ProtocolVersion::V_2025_11_25, false),
        Standard::new(ProtocolVersion::V_2025_06_18, false),
        Standard::new(ProtocolVersion::V_2025_03_26, false),
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Exact(standard) => standard.revision.as_str(),
        }
    }

    pub fn startup_operation(self) -> &'static str {
        match self {
            Self::Auto => "session/startup",
            Self::Exact(v) if v.revision.has_initialize() => "initialize",
            Self::Exact(_) => "server/discover",
        }
    }

    pub fn accepts(self, negotiated: &ProtocolVersion) -> bool {
        match self {
            Self::Auto => Self::REVISIONS.iter().any(|s| &s.revision == negotiated),
            Self::Exact(requested) => &requested.revision == negotiated,
        }
    }

    pub fn all() -> Vec<Self> {
        Self::REVISIONS.iter().map(Self::Exact).collect()
    }

    pub fn validate_matrix(revisions: &[Self]) -> Result<()> {
        ensure!(
            revisions.len() <= Self::REVISIONS.len(),
            "too many MCP revisions"
        );
        let mut seen = BTreeSet::new();
        for r in revisions {
            ensure!(seen.insert(r.as_str()), "duplicate MCP revision");
            ensure!(
                revisions.len() == 1 || *r != Self::Auto,
                "auto must be selected alone"
            );
        }
        Ok(())
    }
}

impl std::str::FromStr for Protocol {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "auto" {
            return Ok(Self::Auto);
        }
        Self::REVISIONS
            .iter()
            .find(|v| v.revision.as_str() == value)
            .map(Self::Exact)
            .ok_or_else(|| {
                format!(
                    "choose auto or one of: {}",
                    Self::REVISIONS
                        .iter()
                        .map(|s| s.revision.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

impl<'de> Deserialize<'de> for Protocol {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    pub users: u32,
    #[serde(default = "default_threads")]
    pub threads: usize,
    #[serde(default, deserialize_with = "deserialize_wait")]
    pub wait: Duration,
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}

fn default_threads() -> usize {
    2
}

fn parse_duration(text: &str) -> Result<Duration, humantime::DurationError> {
    humantime::parse_duration(if text == "-0" { "0" } else { text })
}

fn deserialize_wait<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wait {
        Number(u64),
        Text(String),
    }
    match Wait::deserialize(d)? {
        Wait::Number(0) => Ok(Duration::ZERO),
        Wait::Text(text) => parse_duration(&text).map_err(serde::de::Error::custom),
        Wait::Number(_) => Err(serde::de::Error::custom("use 0 or a duration with units")),
    }
}

pub fn cli_millis(text: &str) -> Result<u64, String> {
    let d = parse_duration(text).map_err(|e| e.to_string())?;
    if !d.subsec_nanos().is_multiple_of(1_000_000) {
        return Err("use a duration in whole milliseconds".into());
    }
    d.as_millis()
        .try_into()
        .map_err(|_| "duration too large".into())
}

#[derive(clap::Args, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StartOptions {
    /// Workload report formats to save (repeat or comma-separate; default: html,json,csv,junit).
    #[arg(
        long = "format",
        value_name = "FORMAT",
        value_enum,
        value_delimiter = ','
    )]
    pub report_formats: Option<Vec<crate::report::Format>>,
    /// One named target.endpoints entry, or "default" for target.url.
    #[arg(long, value_name = "NAME")]
    pub endpoint: Option<String>,
    /// MCP protocol revision for this run, or "auto" to negotiate.
    #[arg(long, value_name = "REVISION")]
    pub protocol: Option<Protocol>,
    /// Concurrent sessions (1-8).
    #[arg(long, value_name = "N")]
    pub users: Option<u32>,
    /// Starling load-generator threads (1-8).
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
    /// Planned run length, from 1s to 7d (e.g. 30s, 6h, 2d).
    #[arg(long = "duration", value_name = "TIME", value_parser = cli_millis)]
    pub duration_ms: Option<u64>,
    /// Optional per-user pause after a request. 0 means no pause (default); maximum 60s.
    #[arg(long = "wait", value_name = "TIME", value_parser = cli_millis, allow_negative_numbers = true)]
    pub wait_ms: Option<u64>,
    /// Per-operation timeout, from 50ms to 15s.
    #[arg(long = "timeout", value_name = "TIME", value_parser = cli_millis)]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub name: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub method: Method,
    pub tool: Option<String>,
    pub prompt: Option<String>,
    pub weight: u32,
    #[serde(default)]
    pub arguments: Map<String, Value>,
    pub assert_exists: Option<String>,
    #[serde(default, deserialize_with = "crate::acceptance::specified")]
    pub assert_match: Option<crate::assertions::MatchAssertion>,
}

#[derive(Clone, Copy, Default, PartialEq, Deserialize)]
pub enum Method {
    #[default]
    #[serde(rename = "tools/call")]
    CallTool,
    #[serde(rename = "tools/list")]
    ListTools,
    #[serde(rename = "prompts/get")]
    GetPrompt,
    #[serde(rename = "prompts/list")]
    ListPrompts,
}

impl Task {
    pub fn operation(&self) -> String {
        let mut key = match self.method {
            Method::CallTool => format!(
                "tools/call/{}",
                self.tool.as_deref().expect("validated tool")
            ),
            Method::ListTools => "task/tools/list".into(),
            Method::GetPrompt => format!(
                "task/prompts/get/{}",
                self.prompt.as_deref().expect("validated prompt")
            ),
            Method::ListPrompts => "task/prompts/list".into(),
        };
        if let Some(name) = &self.name {
            key.push('/');
            key.push_str(name);
        }
        key
    }

    fn validate_request(&self) -> Result<()> {
        ensure!(
            self.title
                .as_deref()
                .is_none_or(crate::assertions::public_text),
            "task.title must be 1-200 printable bytes"
        );
        if let Some(assertion) = &self.assert_match {
            assertion.validate()?;
        }
        ensure!(
            self.name.as_deref().is_none_or(valid_label),
            "task.name must be a public 1-64 character identifier"
        );
        match self.method {
            Method::CallTool => ensure!(
                self.tool.as_deref().is_some_and(valid_label) && self.prompt.is_none(),
                "tools/call requires a valid tool name and no prompt"
            ),
            Method::GetPrompt => {
                ensure!(
                    self.prompt.as_deref().is_some_and(valid_label) && self.tool.is_none(),
                    "prompts/get requires a valid prompt name and no tool"
                );
                ensure!(
                    self.arguments.values().all(Value::is_string),
                    "prompt arguments must be strings"
                );
            }
            Method::ListTools | Method::ListPrompts => ensure!(
                self.tool.is_none() && self.prompt.is_none() && self.arguments.is_empty(),
                "list requests do not take a tool, prompt, or arguments"
            ),
        }
        Ok(())
    }
}

fn invalid_document(format: &str, line: Option<usize>) -> anyhow::Error {
    // Parser diagnostics may contain private questions or tool arguments.
    let at = line.map(|n| format!(" at line {n}")).unwrap_or_default();
    anyhow::anyhow!("invalid workload {format}{at}: check field names, values and types")
}

impl Config {
    fn bundled() -> Result<Self> {
        toml::from_str(STARTER_TOML).context("reading the bundled workload")
    }

    pub fn check_target(url: Url) -> Result<Self> {
        let mut cfg = Self::bundled()?;
        cfg.target.url = url;
        cfg.target.protocol = Protocol::Auto;
        // A direct check has no workload timeout; allow for DNS, TLS and remote startup.
        cfg.load.timeout = MAX_OPERATION_TIMEOUT;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn demo(fixture: SocketAddr, options: StartOptions) -> Result<Self> {
        let mut cfg = Self::bundled()?;
        cfg.run.name = "Starling demo".into();
        cfg.target.url = Url::parse(&format!("http://{fixture}/mcp"))?;
        cfg.with_options(options)
    }

    pub fn with_options(mut self, options: StartOptions) -> Result<Self> {
        if let Some(formats) = options.report_formats {
            self.report.formats = formats.into_iter().collect();
        }
        if let Some(name) = options.endpoint {
            self.target.selected = Some(name);
        }
        if let Some(protocol) = options.protocol {
            self.target.protocol = protocol;
        }
        if let Some(users) = options.users {
            self.load.users = users;
        }
        if let Some(threads) = options.threads {
            self.load.threads = threads;
        }
        if let Some(ms) = options.duration_ms {
            self.run.duration = Duration::from_millis(ms);
        }
        if let Some(ms) = options.wait_ms {
            self.load.wait = Duration::from_millis(ms);
        }
        if let Some(ms) = options.timeout_ms {
            self.load.timeout = Duration::from_millis(ms);
        }
        self.validate()?;
        Ok(self)
    }

    pub fn read(path: &Path, options: StartOptions) -> Result<Self> {
        let mut text = String::new();
        File::open(path)
            .and_then(|f| {
                f.take(MAX_CONFIG_BYTES as u64 + 1)
                    .read_to_string(&mut text)
            })
            .with_context(|| format!("reading workload {}", path.display()))?;
        ensure!(text.len() <= MAX_CONFIG_BYTES, "workload exceeds 64 KiB");
        if is_yaml(path) {
            return Self::from_yaml(&text, options);
        }
        let cfg: Self = toml::from_str(&text).map_err(|e: toml::de::Error| {
            invalid_document(
                "TOML",
                e.span()
                    .map(|s| 1 + text.bytes().take(s.start).filter(|&b| b == b'\n').count()),
            )
        })?;
        cfg.with_options(options)
    }

    pub fn from_yaml(text: &str, options: StartOptions) -> Result<Self> {
        ensure!(text.len() <= MAX_CONFIG_BYTES, "workload exceeds 64 KiB");
        let cfg: Self = serde_yaml_ng::from_str(text)
            .map_err(|e| invalid_document("YAML", e.location().map(|loc| loc.line())))?;
        cfg.with_options(options)
    }

    fn validate(&mut self) -> Result<()> {
        ensure!(
            !self.report.formats.is_empty(),
            "select at least one report format"
        );
        if let Some(criteria) = &self.acceptance {
            criteria.validate()?;
        }
        Protocol::validate_matrix(&self.conformance.revisions)?;
        ensure!(
            !self.run.name.trim().is_empty()
                && self.run.name.len() <= 80
                && !self.run.name.chars().any(char::is_control),
            "run.name must be 1-80 printable bytes"
        );
        ensure!(
            (Duration::from_secs(1)..=MAX_DURATION).contains(&self.run.duration),
            "run.duration must be 1s-7d"
        );
        ensure!((1..=8).contains(&self.load.users), "load.users must be 1-8");
        ensure!(
            (1..=8).contains(&self.load.threads),
            "load.threads must be 1-8"
        );
        ensure!(
            self.load.wait.is_zero()
                || (Duration::from_millis(1)..=Duration::from_secs(60)).contains(&self.load.wait),
            "load.wait must be 0 (no pause) or 1ms-60s"
        );
        ensure!(
            (Duration::from_millis(50)..=MAX_OPERATION_TIMEOUT).contains(&self.load.timeout),
            "load.timeout must be 50ms-15s"
        );
        validate_url(&mut self.target.url)?;
        self.target.selected.take_if(|name| name == "default");
        ensure!(
            self.target
                .selected
                .as_ref()
                .is_none_or(|name| self.target.endpoints.contains_key(name)),
            "unknown endpoint: choose default or a name listed in target.endpoints"
        );
        ensure!(
            self.target.endpoints.len() <= 8,
            "configure at most 8 alternative endpoints"
        );
        for (name, url) in &mut self.target.endpoints {
            ensure!(
                valid_label(name) && name != "default",
                "endpoint names must be public identifiers other than default"
            );
            validate_url(url)?;
        }
        let max_tasks = if self.run.mode == RunMode::Once {
            64
        } else {
            16
        };
        ensure!(
            (1..=max_tasks).contains(&self.tasks.len()),
            "configure 1-{max_tasks} tasks"
        );
        let mut names = BTreeSet::new();
        for task in &self.tasks {
            task.validate_request()?;
            ensure!(
                names.insert(task.operation()),
                "duplicate request: give repeated cases distinct task names"
            );
            ensure!(
                (1..=1000).contains(&task.weight),
                "task weights must be 1-1000"
            );
            ensure!(
                self.run.mode != RunMode::Once || task.weight == 1,
                "once mode requires weight 1 for every case"
            );
            if let Some(pointer) = &task.assert_exists {
                ensure!(
                    crate::assertions::valid_pointer(pointer),
                    "assert_exists must be a JSON Pointer, e.g. /content/0/text"
                );
            }
        }
        Ok(())
    }

    pub fn pick<'a>(&'a self, rng: &mut fastrand::Rng) -> &'a Task {
        let mut ticket = rng.u32(..self.tasks.iter().map(|t| t.weight).sum::<u32>());
        for task in &self.tasks {
            if ticket < task.weight {
                return task;
            }
            ticket -= task.weight;
        }
        unreachable!("validated nonempty weight distribution")
    }
}

pub fn loopback_addr(text: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = text.parse().map_err(|_| "expected IP:port")?;
    if addr.ip().is_loopback() {
        Ok(addr)
    } else {
        Err("listen address must be loopback".into())
    }
}

#[cfg(test)]
mod tests;
