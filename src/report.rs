use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{
    metrics::Outcome,
    runner::{Frame, Status},
};

pub const DASHBOARD: &str = include_str!("../web/dist/index.html");
const DATA: &str = r#"<script id="starling-report" type="application/json">null</script>"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Html,
    Json,
    Yaml,
    Csv,
    Junit,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Csv => "csv",
            Self::Junit => "junit",
        }
    }

    fn files(self) -> &'static [Artifact] {
        match self {
            Self::Html => &[Artifact::Dashboard],
            Self::Json => &[Artifact::Summary, Artifact::Run, Artifact::Timeseries],
            Self::Yaml => &[Artifact::YamlSummary],
            Self::Csv => &[Artifact::Operations, Artifact::Errors],
            Self::Junit => &[Artifact::Junit],
        }
    }

    pub fn primary(self) -> Artifact {
        self.files()[0]
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReportOptions {
    pub formats: BTreeSet<Format>,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            formats: [Format::Html, Format::Json, Format::Csv, Format::Junit].into(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Artifact {
    Dashboard,
    Run,
    Summary,
    YamlSummary,
    Timeseries,
    Operations,
    Errors,
    Junit,
}

impl Artifact {
    pub fn named(name: &str) -> Option<Self> {
        Format::value_variants()
            .iter()
            .flat_map(|format| format.files())
            .copied()
            .find(|file| file.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Dashboard => "index.html",
            Self::Run => "run.json",
            Self::Summary => "summary.json",
            Self::YamlSummary => "summary.yaml",
            Self::Timeseries => "timeseries.json",
            Self::Operations => "operations.csv",
            Self::Errors => "errors.csv",
            Self::Junit => "junit.xml",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Dashboard => "text/html; charset=utf-8",
            Self::Run | Self::Summary | Self::Timeseries => "application/json",
            Self::YamlSummary => "application/yaml",
            Self::Operations | Self::Errors => "text/csv; charset=utf-8",
            Self::Junit => "application/xml",
        }
    }

    pub fn render(self, frame: &Frame) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Dashboard => html(frame)?.into_bytes(),
            Self::Run => serde_json::to_vec_pretty(&frame.view.run)?,
            Self::Summary => serde_json::to_vec_pretty(&frame.view)?,
            Self::YamlSummary => serde_yaml_ng::to_string(&frame.view)?.into_bytes(),
            Self::Timeseries => serde_json::to_vec_pretty(&frame.series)?,
            Self::Operations => operations_csv(frame)?.into_bytes(),
            Self::Errors => errors_csv(frame)?.into_bytes(),
            Self::Junit => junit(frame)?.into_bytes(),
        })
    }
}

pub fn primary_path(dir: &Path, frame: &Frame) -> PathBuf {
    let format = frame
        .view
        .run
        .report_formats
        .first()
        .expect("validated report formats");
    dir.join(format.primary().name())
}

pub fn reserve(path: Option<PathBuf>) -> Result<PathBuf> {
    let Some(dir) = path else {
        return reserve_in(Path::new("out"));
    };
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&dir)
        .with_context(|| format!("report directory must be new: {}", dir.display()))?;
    Ok(dir)
}

pub fn reserve_in(parent: &Path) -> Result<PathBuf> {
    fs::create_dir_all(parent)?;
    let base = format!(
        "run-{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        std::process::id()
    );
    let mut dir = parent.join(&base);
    for suffix in 1.. {
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                dir = parent.join(format!("{base}-{suffix}"));
            }
            Err(e) => return Err(e).context("reserving report directory"),
        }
    }
    unreachable!()
}

fn html(frame: &Frame) -> Result<String> {
    ensure!(
        DASHBOARD.contains(DATA),
        "embedded dashboard is missing its report data slot"
    );
    let json = serde_json::to_string(frame)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    Ok(DASHBOARD.replacen(
        DATA,
        &format!(r#"<script id="starling-report" type="application/json">{json}</script>"#),
        1,
    ))
}

fn file(dir: &Path, name: &str, data: &[u8]) -> Result<()> {
    fs::File::create_new(dir.join(name))
        .with_context(|| format!("creating {name}"))?
        .write_all(data)
        .with_context(|| format!("writing {name}"))?;
    Ok(())
}

pub fn json(dir: &Path, name: &str, data: &impl Serialize) -> Result<()> {
    file(dir, name, &serde_json::to_vec_pretty(data)?)
}

pub fn write(dir: &Path, frame: &Frame) -> Result<()> {
    for format in [
        Format::Json,
        Format::Yaml,
        Format::Csv,
        Format::Junit,
        Format::Html,
    ] {
        if frame.view.run.report_formats.contains(&format) {
            for artifact in format.files() {
                file(dir, artifact.name(), &artifact.render(frame)?)?;
            }
        }
    }
    Ok(())
}

fn operations_csv(frame: &Frame) -> Result<String> {
    let mut ops = String::from("operation,count,failed,p50_ms,p95_ms,p99_ms\n");
    for (name, s) in &frame.view.operations {
        let ms = |v: Option<f64>| v.map(|v| format!("{v:.3}")).unwrap_or_default();
        writeln!(
            ops,
            "{name},{},{},{},{},{}",
            s.count,
            s.failed,
            ms(s.p50_ms),
            ms(s.p95_ms),
            ms(s.p99_ms)
        )?;
    }
    Ok(ops)
}

fn errors_csv(frame: &Frame) -> Result<String> {
    let mut errors = String::from("operation,category,count\n");
    for (name, s) in &frame.view.operations {
        for (&outcome, count) in &s.outcomes {
            if outcome != Outcome::Success {
                writeln!(errors, "{name},{},{count}", outcome.label())?;
            }
        }
    }
    Ok(errors)
}

fn junit(frame: &Frame) -> Result<String> {
    let mut cases = String::new();
    let mut failures = 0;
    let mut skipped = 0;
    for (name, s) in &frame.view.operations {
        write!(cases, "  <testcase name=\"{name}\">")?;
        if s.failed != 0 {
            failures += 1;
            write!(
                cases,
                "<failure message=\"{} of {} operations failed\"/>",
                s.failed, s.count
            )?;
        }
        cases.push_str("</testcase>\n");
    }
    for check in &frame.view.assertions {
        if check.status == crate::runner::CaseStatus::NotRun {
            writeln!(
                cases,
                "  <testcase name=\"{}\"><skipped message=\"Case did not run\"/></testcase>",
                check.operation
            )?;
            skipped += 1;
        }
    }
    if let Some(gate) = &frame.view.acceptance {
        cases.push_str("  <testcase name=\"performance.acceptance\">");
        if let Some(reason) = gate.failure() {
            failures += 1;
            write!(
                cases,
                "<failure message=\"{}\"/>",
                reason
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('"', "&quot;")
            )?;
        }
        cases.push_str("</testcase>\n");
    }
    let infrastructure_failure = failures == 0 && frame.view.status == Status::Failed;
    if infrastructure_failure {
        failures += 1;
        cases.push_str("  <testcase name=\"run\"><failure message=\"Run failed\"/></testcase>\n");
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"starling\" tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\">\n{cases}</testsuite>\n",
        frame.view.operations.len()
            + skipped
            + usize::from(frame.view.acceptance.is_some())
            + usize::from(infrastructure_failure)
    ))
}
