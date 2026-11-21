use std::{
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use crate::{
    metrics::Outcome,
    runner::{Frame, Status},
};

pub const DASHBOARD: &str = include_str!("../web/dist/index.html");
const DATA: &str = r#"<script id="starling-report" type="application/json">null</script>"#;

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

pub fn html(frame: &Frame) -> Result<String> {
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
    fs::File::create_new(dir.join(name))?
        .write_all(data)
        .with_context(|| format!("writing {name}"))?;
    Ok(())
}

pub fn json(dir: &Path, name: &str, data: &impl Serialize) -> Result<()> {
    file(dir, name, &serde_json::to_vec_pretty(data)?)
}

pub fn write(dir: &Path, frame: &Frame) -> Result<()> {
    json(dir, "run.json", &frame.view.run)?;
    json(dir, "summary.json", &frame.view)?;
    json(dir, "timeseries.json", &frame.series)?;
    let mut ops = String::from("operation,count,failed,p50_ms,p95_ms,p99_ms\n");
    let mut errors = String::from("operation,category,count\n");
    let mut cases = String::new();
    let mut failures = 0;
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
        for (&outcome, count) in &s.outcomes {
            if outcome != Outcome::Success {
                writeln!(errors, "{name},{},{count}", outcome.label())?;
            }
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
        cases.push_str("  <testcase name=\"run\"><failure message=\"Run failed; see summary.json\"/></testcase>\n");
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"starling\" tests=\"{}\" failures=\"{failures}\">\n{cases}</testsuite>\n",
        frame.view.operations.len()
            + usize::from(frame.view.acceptance.is_some())
            + usize::from(infrastructure_failure)
    );
    file(dir, "operations.csv", ops.as_bytes())?;
    file(dir, "errors.csv", errors.as_bytes())?;
    file(dir, "junit.xml", xml.as_bytes())?;
    file(dir, "index.html", html(frame)?.as_bytes())
}
