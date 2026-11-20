use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Map, Value};
use url::{Host, Url};

pub const MAX_DURATION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const DEFAULT_PATH: &str = "starling.yaml";
const STARTER_YAML: &str = include_str!("../examples/local.yaml");

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
        include_str!("../examples/local.toml")
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
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub name: String,
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    pub seed: u64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub url: Url,
    #[serde(default)]
    pub protocol: Protocol,
}

#[derive(Clone, Copy, Default, Deserialize)]
pub enum Protocol {
    #[serde(rename = "2025-11-25")]
    Initialize,
    #[default]
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    pub users: u32,
    #[serde(default = "default_threads")]
    pub threads: usize,
    #[serde(with = "humantime_serde")]
    pub wait: Duration,
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}

fn default_threads() -> usize {
    2
}

fn cli_millis(text: &str) -> Result<u64, String> {
    let d = humantime::parse_duration(text).map_err(|e| e.to_string())?;
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
    /// Concurrent sessions (1-8).
    #[arg(long, value_name = "N")]
    pub users: Option<u32>,
    /// Starling load-generator threads (1-8).
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
    /// Planned run length, from 1s to 7d (e.g. 30s, 6h, 2d).
    #[arg(long = "duration", value_name = "TIME", value_parser = cli_millis)]
    pub duration_ms: Option<u64>,
    /// Wait after each tool call, from 200ms to 60s.
    #[arg(long = "wait", value_name = "TIME", value_parser = cli_millis)]
    pub wait_ms: Option<u64>,
    /// Per-operation timeout, from 50ms to 15s.
    #[arg(long = "timeout", value_name = "TIME", value_parser = cli_millis)]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub tool: String,
    pub weight: u32,
    #[serde(default)]
    pub arguments: Map<String, Value>,
    pub assert_exists: Option<String>,
}

impl Config {
    pub fn demo(fixture: SocketAddr, options: StartOptions) -> Result<Self> {
        let mut cfg: Self =
            serde_yaml_ng::from_str(STARTER_YAML).context("reading the bundled demo")?;
        cfg.run.name = "Starling demo".into();
        cfg.target.url = Url::parse(&format!("http://{fixture}/mcp"))?;
        cfg.with_options(options)
    }

    pub fn with_options(mut self, options: StartOptions) -> Result<Self> {
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
            .and_then(|f| f.take(65537).read_to_string(&mut text))
            .with_context(|| format!("reading workload {}", path.display()))?;
        ensure!(text.len() <= 65536, "workload exceeds 64 KiB");
        // Parser diagnostics can quote input, including private tool arguments.
        let invalid = |format: &str, line: Option<usize>| {
            let at = line.map(|n| format!(" at line {n}")).unwrap_or_default();
            anyhow::anyhow!("invalid workload {format}{at}: check field names, values and types")
        };
        let cfg: Self = if is_yaml(path) {
            serde_yaml_ng::from_str(&text)
                .map_err(|e| invalid("YAML", e.location().map(|loc| loc.line())))?
        } else {
            toml::from_str(&text).map_err(|e: toml::de::Error| {
                invalid(
                    "TOML",
                    e.span()
                        .map(|s| 1 + text.bytes().take(s.start).filter(|&b| b == b'\n').count()),
                )
            })?
        };
        cfg.with_options(options)
    }

    fn validate(&mut self) -> Result<()> {
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
            (Duration::from_millis(200)..=Duration::from_secs(60)).contains(&self.load.wait),
            "load.wait must be 200ms-60s"
        );
        ensure!(
            (Duration::from_millis(50)..=Duration::from_secs(15)).contains(&self.load.timeout),
            "load.timeout must be 50ms-15s"
        );
        let url = &mut self.target.url;
        ensure!(
            url.scheme() == "http"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "target must use http without credentials, query or fragment"
        );
        match url.host() {
            Some(Host::Ipv4(ip)) if ip.is_loopback() => (),
            Some(Host::Ipv6(ip)) if ip.is_loopback() => (),
            Some(Host::Domain("localhost")) => url.set_host(Some("127.0.0.1"))?,
            _ => bail!("target must be a loopback IP or localhost"),
        }
        ensure!((1..=16).contains(&self.tasks.len()), "configure 1-16 tasks");
        let mut names = BTreeSet::new();
        for task in &self.tasks {
            ensure!(
                task.tool.len() <= 64
                    && task.tool.starts_with(|c: char| c.is_ascii_alphanumeric())
                    && task
                        .tool
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
                "invalid tool name: use 1-64 letters, digits, '.', '-' or '_'"
            );
            ensure!(names.insert(&task.tool), "duplicate tool: {}", task.tool);
            ensure!(
                (1..=1000).contains(&task.weight),
                "task weights must be 1-1000"
            );
            if let Some(pointer) = &task.assert_exists {
                ensure!(
                    pointer.starts_with('/')
                        && pointer.len() <= 256
                        && pointer
                            .split('~')
                            .skip(1)
                            .all(|s| s.starts_with(['0', '1'])),
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
mod tests {
    use super::*;

    fn sample() -> Config {
        toml::from_str(include_str!("../examples/local.toml")).unwrap()
    }

    #[test]
    fn local_limits_and_weights_are_checked_before_dispatch() {
        for url in [
            "http://192.0.2.1/mcp",
            "http://localhost.example/mcp",
            "http://u:p@localhost/mcp",
            "http://localhost/mcp?token=abc",
        ] {
            let mut cfg = sample();
            cfg.target.url = url.parse().unwrap();
            assert!(cfg.validate().is_err());
        }
        let mut cfg = sample();
        cfg.target.url = "http://localhost:9000/mcp".parse().unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.target.url.host_str(), Some("127.0.0.1"));
        cfg.tasks[0].weight = 0;
        assert!(cfg.validate().is_err());
        assert!(loopback_addr("0.0.0.0:9000").is_err());
    }

    #[test]
    fn selection_is_seeded_and_respects_weights() {
        let cfg = sample();
        let sequence = || {
            let mut rng = fastrand::Rng::with_seed(42);
            (0..1000)
                .map(|_| cfg.pick(&mut rng).tool.as_str())
                .collect::<Vec<_>>()
        };
        let draws = sequence();
        assert_eq!(draws, sequence());
        assert!((680..820).contains(&draws.iter().filter(|&&s| s == "echo").count()));
    }
}
