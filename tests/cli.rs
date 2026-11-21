use std::{fs, path::Path, process::Stdio, time::Duration};

use serde_json::Value;
use tempfile::{TempDir, tempdir};
use tokio::{
    io::{AsyncBufReadExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
    time::timeout,
};

const BIN: &str = env!("CARGO_BIN_EXE_starling");
const SAMPLE: &str = include_str!("../examples/local.toml");

#[path = "cli/conformance.rs"]
mod conformance;
#[path = "cli/protocol.rs"]
mod protocol;
#[path = "cli/telemetry.rs"]
mod telemetry;

fn starling() -> Command {
    let mut command = Command::new(BIN);
    command
        .env_remove("STARLING_TELEMETRY")
        .env_remove("STARLING_LOG_LEVEL");
    command
}

async fn announced(lines: &mut Lines<BufReader<ChildStdout>>, prefix: &str) -> String {
    while let Some(line) = lines.next_line().await.unwrap() {
        if let Some(value) = line.strip_prefix(prefix) {
            return value.to_owned();
        }
    }
    panic!("process exited before printing {prefix}");
}

async fn fixture() -> (Child, String) {
    let mut child = starling()
        .args(["fixture", "--listen", "127.0.0.1:0"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
    let url = timeout(Duration::from_secs(8), announced(&mut reader, "Fixture: "))
        .await
        .unwrap();
    (child, url)
}

async fn run(config: String, expect_success: bool) -> (TempDir, Value) {
    run_file(config, "test.toml", &[], expect_success).await
}

async fn run_file(
    config: String,
    name: &str,
    overrides: &[&str],
    expect_success: bool,
) -> (TempDir, Value) {
    let work = tempdir().unwrap();
    fs::write(work.path().join(name), config).unwrap();
    let output = timeout(
        Duration::from_secs(12),
        starling()
            .current_dir(work.path())
            .args(["run", "--headless", "--config", name, "--report", "report"])
            .args(overrides)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    let report = work.path().join("report");
    let summary = read(&report, "summary.json");
    assert_eq!(
        output.status.success(),
        expect_success,
        "stderr={} summary={summary}",
        String::from_utf8_lossy(&output.stderr)
    );
    (work, summary)
}

fn read(dir: &Path, file: &str) -> Value {
    serde_json::from_slice(&fs::read(dir.join(file)).unwrap()).unwrap()
}

async fn snapshot(http: &reqwest::Client, url: &str) -> Value {
    let bytes = http
        .get(url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn dashboard_client() -> reqwest::Client {
    #[cfg(feature = "otlp")]
    {
        static TLS: std::sync::Once = std::sync::Once::new();
        TLS.call_once(|| {
            rustls::crypto::ring::default_provider()
                .install_default()
                .expect("test TLS provider")
        });
    }
    reqwest::Client::builder().no_proxy().build().unwrap()
}

fn config(url: &str) -> String {
    SAMPLE
        .replace("http://127.0.0.1:8081/mcp", url)
        .replace("30s", "2s")
}

#[test]
fn init_produces_usable_workloads_outside_the_checkout_and_preserves_user_files() {
    let work = tempdir().unwrap();
    let cli = |args: &[&str]| {
        std::process::Command::new(BIN)
            .current_dir(work.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(cli(&["init"]).status.success());
    assert!(cli(&["validate"]).status.success());
    let default = work.path().join("starling.yaml");
    fs::write(&default, "my existing workload").unwrap();
    assert!(!cli(&["init"]).status.success());
    assert_eq!(fs::read_to_string(default).unwrap(), "my existing workload");
    assert!(cli(&["init", "custom.toml"]).status.success());
    assert!(
        cli(&["validate", "--config", "custom.toml"])
            .status
            .success()
    );
    assert!(!cli(&["init", "custom.txt"]).status.success());
    assert!(!work.path().join("custom.txt").exists());
}

#[tokio::test]
async fn standalone_demo_runs_without_a_file_or_external_fixture() {
    let work = tempdir().unwrap();
    let output = timeout(
        Duration::from_secs(12),
        starling()
            .current_dir(work.path())
            .args(["demo", "--duration", "1s", "--report", "demo-report"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dir = work.path().join("demo-report");
    let s = read(&dir, "summary.json");
    assert_eq!(s["status"], "completed");
    assert_eq!(s["run"]["wait_ms"], 0);
    assert!(s["calls"]["count"].as_u64().unwrap() > 0);
    assert_eq!(s["calls"]["failed"], 0);
    assert!(dir.join("index.html").is_file());
    assert!(!work.path().join("starling.yaml").exists());
}

#[test]
fn malformed_workload_reports_location_without_echoing_arguments() {
    let work = tempdir().unwrap();
    let path = work.path().join("bad.toml");
    fs::write(
        &path,
        SAMPLE.replace(
            "arguments = { text = \"hello\" }",
            "arguments = 'PRIVATE_INVALID_VALUE'",
        ),
    )
    .unwrap();
    let output = std::process::Command::new(BIN)
        .args(["validate", "--config"])
        .arg(path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid workload TOML at line"));
    assert!(!stderr.contains("PRIVATE_INVALID_VALUE"));
}

#[tokio::test]
async fn failed_bird_reports_its_final_counts_without_starting_the_controller() {
    let work = tempdir().unwrap();
    timeout(Duration::from_secs(15), async {
        let mut controller = starling()
            .current_dir(work.path())
            .args(["demo", "--web", "--report", "controller"])
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(controller.stdout.take().unwrap()).lines();
        let target = announced(&mut lines, "Demo fixture: ").await;
        let url = announced(&mut lines, "Dashboard: ").await;
        let home = announced(&mut lines, "Bird telemetry: ").await;
        let yaml = format!(
            "run: {{ name: Bird failure, duration: 1s, seed: 42 }}\n\
            target: {{ url: '{target}', protocol: auto }}\n\
            load: {{ users: 1, wait: 250ms, timeout: 2s }}\n\
            tasks: [{{ tool: status, weight: 1, assert_exists: /missing }}]\n"
        );
        fs::write(work.path().join("bird.yaml"), yaml).unwrap();
        let out = starling()
            .current_dir(work.path())
            .args([
                "bird",
                "--id",
                "wing",
                "--home",
                &home,
                "--config",
                "bird.yaml",
                "--protocol",
                "2025-03-26",
                "--report",
                "wing",
            ])
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report = read(&work.path().join("wing"), "summary.json");
        assert_eq!(report["run"]["protocol"], "2025-03-26");
        assert_eq!(report["protocols"], serde_json::json!(["2025-03-26"]));
        let failed = report["calls"]["failed"].as_u64().unwrap();
        assert!(failed > 0);
        let end = String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|row| row["event"] == "run_finished")
            .expect("missing bird completion log");
        assert_eq!(end["role"], "bird");
        assert_eq!(end["birdId"], "wing");
        assert_eq!(end["status"], "failed");
        assert_eq!(end["failed"], failed);
        assert_eq!(
            report["operations"]["tools/call/status"]["outcomes"]["assertion"],
            failed
        );
        let http = dashboard_client();
        let roster = snapshot(&http, &format!("{url}/api/v1/flock")).await;
        let birds = roster["birds"].as_array().unwrap();
        assert_eq!(birds.len(), 1);
        assert_eq!(birds[0]["id"], "wing");
        assert_eq!(birds[0]["state"], "failed");
        assert_eq!(birds[0]["requests"], report["calls"]["count"]);
        assert_eq!(birds[0]["failed"], failed);
        let local = snapshot(&http, &format!("{url}/api/v1/runs/local")).await;
        assert_eq!(local["status"], "ready");
        assert_eq!(local["calls"]["count"], 0);
        controller.kill().await.unwrap();
    })
    .await
    .expect("bird did not finish and report its failure");
}

#[tokio::test]
async fn dashboard_fails_without_overwriting_an_existing_report_file() {
    let work = tempdir().unwrap();
    timeout(Duration::from_secs(12), async {
        let mut child = starling()
            .current_dir(work.path())
            .args([
                "demo",
                "--web",
                "--duration",
                "1s",
                "--wait",
                "200ms",
                "--report",
                "report",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let url = announced(&mut lines, "Dashboard: ").await;
        let html = work.path().join("report/index.html");
        fs::write(&html, b"existing contents").unwrap();
        let response = dashboard_client()
            .post(format!("{url}/api/v1/runs/local/start"))
            .header("X-Starling-Control", "1")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 202);
        let output = child.wait_with_output().await.unwrap();
        assert_eq!(output.status.code(), Some(1));
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("writing report bundle"), "{error}");
        assert_eq!(fs::read(html).unwrap(), b"existing contents");
    })
    .await
    .expect("dashboard did not exit after export failed");
}

#[test]
fn yaml_validation_accepts_long_durations_and_keeps_parse_errors_private() {
    let tmp = tempdir().unwrap();
    let file = tmp.path().join("load.yaml");
    let yaml = include_str!("../examples/local.yaml");
    let validate = |duration| {
        std::process::Command::new(BIN)
            .args(["validate", "--config"])
            .arg(&file)
            .args(["--duration", duration])
            .output()
            .unwrap()
    };
    fs::write(&file, yaml).unwrap();
    assert!(validate("7d").status.success());
    assert!(!validate("7d 1ms").status.success());
    fs::write(
        &file,
        yaml.replace("users: 2", "users: PRIVATE_INVALID_VALUE"),
    )
    .unwrap();
    let result = validate("6h");
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(
        error.contains("invalid workload YAML at line") && !error.contains("PRIVATE_INVALID_VALUE")
    );
}

#[tokio::test]
async fn yaml_cli_overrides_drive_the_run_and_persist_without_editing_the_file() {
    let (mut child, target) = fixture().await;
    let yaml = format!(
        r#"
run: {{ name: CLI example, duration: 6h, seed: 42 }}
target: {{ url: "{target}", protocol: "2025-11-25" }}
load: {{ users: 1, wait: 1s, timeout: 12s }}
tasks:
  - tool: echo
    weight: 1
    arguments: {{ text: sample }}
    assert_exists: /content/0/text
"#
    );
    let (dir, s) = run_file(
        yaml.clone(),
        "work.yml",
        &[
            "--duration",
            "1.5s",
            "--users",
            "3",
            "--threads",
            "1",
            "--wait",
            "0",
            "--timeout",
            "2s",
        ],
        true,
    )
    .await;
    let meta = read(&dir.path().join("report"), "run.json");
    for (field, n) in [
        ("duration_ms", 1500),
        ("users", 3),
        ("threads", 1),
        ("wait_ms", 0),
        ("timeout_ms", 2000),
    ] {
        assert_eq!(meta[field], n);
    }
    assert_eq!(s["operations"]["initialize"]["count"], 3);
    // At a one-second think time, three users could complete at most six calls in 1.5s.
    assert!(s["calls"]["count"].as_u64().unwrap() > 6);
    assert_eq!(s["calls"]["failed"], 0);
    assert_eq!(
        fs::read_to_string(dir.path().join("work.yml")).unwrap(),
        yaml
    );
    child.kill().await.unwrap();
}

#[tokio::test]
async fn named_yaml_cases_use_the_selected_endpoint_and_real_methods() {
    let (mut child, target) = fixture().await;
    let yaml = include_str!("../examples/local.yaml")
        .replace(
            "url: http://127.0.0.1:8081/mcp",
            &format!("url: http://127.0.0.1:1/mcp\n  endpoints: {{ candidate: '{target}' }}"),
        )
        .replace("How do I get started?", "PRIVATE_SCENARIO_QUESTION");
    let yaml = format!(
        "{yaml}\n  - method: prompts/list\n    weight: 1\n    assert_exists: /prompts/0/name\n"
    );
    let (work, s) = run_file(
        yaml,
        "scenario.yaml",
        &[
            "--duration",
            "3s",
            "--wait",
            "200ms",
            "--endpoint",
            "candidate",
        ],
        true,
    )
    .await;
    assert_eq!(s["run"]["endpoint"], "candidate");
    assert!(target.starts_with(s["run"]["target_origin"].as_str().unwrap()));
    assert_eq!(s["operations"]["tools/list"]["count"], 2);
    let cases = [
        "tools/call/echo/getting-started",
        "tools/call/echo/check-status",
        "task/tools/list/catalog",
        "task/prompts/get/ask/ask-question",
        "task/prompts/list",
    ];
    let count: u64 = cases
        .iter()
        .map(|k| {
            let n = s["operations"][k]["count"].as_u64().unwrap();
            assert!(n > 0, "missing measured request {k}");
            n
        })
        .sum();
    assert_eq!(s["calls"]["count"], count);
    assert_eq!(s["calls"]["failed"], 0);
    assert!(
        !fs::read_to_string(work.path().join("report/index.html"))
            .unwrap()
            .contains("PRIVATE_SCENARIO_QUESTION")
    );
    child.kill().await.unwrap();
}

#[tokio::test]
async fn real_sessions_produce_consistent_redacted_offline_reports() {
    let (mut child, url) = fixture().await;
    let (work, s) = run(
        config(&url)
            .replace("wait = 0", "wait = \"250ms\"")
            .replace("threads = 2", "threads = 1")
            .replace("hello", "PRIVATE_TEST_PAYLOAD")
            .replace("Local smoke test", "</script><fixture>"),
        true,
    )
    .await;
    let ops = &s["operations"];
    assert_eq!(s["protocols"][0], "2025-11-25");
    assert_eq!(s["status"], "completed");
    assert_eq!(ops["initialize"]["count"], 2);
    assert_eq!(ops["tools/list"]["count"], 2);
    assert_eq!(s["in_flight"], 0);
    assert_eq!(s["sessions"], 0);
    for name in ["tools/call/echo", "tools/call/status"] {
        assert!(ops[name]["outcomes"]["success"].as_u64().unwrap() > 0);
    }
    let dir = work.path().join("report");
    assert_eq!(read(&dir, "run.json")["threads"], 1);
    assert_eq!(s["run"]["wait_ms"], 250);
    assert_eq!(s["run"]["timeout_ms"], 3000);
    let series = read(&dir, "timeseries.json");
    let count: u64 = series
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["calls"].as_u64().unwrap())
        .sum();
    assert_eq!(s["calls"]["count"], count);
    assert_eq!(s["calls"]["failed"], 0);
    assert_eq!(series[0]["span_ms"], 1000.0);
    for name in [
        "index.html",
        "summary.json",
        "run.json",
        "timeseries.json",
        "operations.csv",
        "errors.csv",
        "junit.xml",
    ] {
        let text = fs::read_to_string(dir.join(name)).unwrap();
        assert!(
            !text.contains("PRIVATE_TEST_PAYLOAD"),
            "{name} contains a tool argument/result"
        );
        if name == "index.html" {
            assert!(!text.contains("</script><fixture>"));
        }
    }
    child.kill().await.unwrap();
}

#[tokio::test]
async fn discovery_assertion_semantic_and_rpc_failures_are_distinct() {
    let (mut child, url) = fixture().await;
    let cfg = config(&url)
        .replace("2s", "1s")
        .replace("users = 2", "users = 1")
        .replace("2025-11-25", "auto");
    let (header, _) = cfg.split_once("[[tasks]]").unwrap();
    for (task, op, category) in [
        ("tool = 'missing'", "tools/list", "capability"),
        ("tool = 'echo'", "tools/call/echo", "json_rpc"),
        (
            "tool = 'status'\nassert_exists = '/absent'",
            "tools/call/status",
            "assertion",
        ),
        ("tool = 'reject'", "tools/call/reject", "tool_error"),
    ] {
        let (work, summary) = run(format!("{header}[[tasks]]\nweight = 1\n{task}\n"), false).await;
        assert_eq!(summary["protocols"][0], "2026-07-28", "{summary}");
        assert_eq!(summary["status"], "failed");
        assert!(
            summary["operations"][op]["outcomes"][category]
                .as_u64()
                .is_some_and(|n| n > 0),
            "{summary}"
        );
        assert!(
            fs::read_to_string(work.path().join("report/errors.csv"))
                .unwrap()
                .contains(&format!("{op},{category},"))
        );
    }
    child.kill().await.unwrap();
}

#[tokio::test]
async fn hung_handshake_times_out_without_dispatching_tools() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_, summary) = run(
        config(&format!("http://{}/mcp", listener.local_addr().unwrap())).replace("3s", "100ms"),
        false,
    )
    .await;
    assert_eq!(summary["calls"]["count"], 0);
    assert!(
        summary["operations"]["initialize"]["outcomes"]["timeout"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

#[cfg(unix)]
mod signals {
    use super::*;

    fn terminate(child: &Child) {
        let sent = std::process::Command::new("sh")
            .args(["-c", "kill -TERM \"$1\"", "signal-test"])
            .arg(child.id().unwrap().to_string())
            .status()
            .unwrap();
        assert!(sent.success());
    }

    #[tokio::test]
    async fn sigterm_drains_the_dashboard_and_its_reporting_bird() {
        let work = tempdir().unwrap();
        let reported = timeout(Duration::from_secs(15), async {
            let mut child = starling()
                .current_dir(work.path())
                .args([
                    "demo",
                    "--web",
                    "--birds",
                    "1",
                    "--duration",
                    "1m",
                    "--wait",
                    "200ms",
                    "--report",
                    "report",
                ])
                .stdout(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
            let url = announced(&mut lines, "Dashboard: ").await;
            let api = format!("{url}/api/v1/runs/local");
            let http = dashboard_client();
            let res = http
                .post(format!("{api}/start"))
                .header("X-Starling-Control", "1")
                .send()
                .await
                .unwrap();
            assert_eq!(res.status(), 202);
            let reported = loop {
                let roster = snapshot(&http, &format!("{url}/api/v1/flock")).await;
                let bird = &roster["birds"][0];
                if bird["id"] == "bird-01"
                    && bird["state"] == "running"
                    && let Some(n) = bird["requests"].as_u64().filter(|n| *n > 0)
                    && snapshot(&http, &api).await["calls"]["count"]
                        .as_u64()
                        .unwrap()
                        > 0
                {
                    break n;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            terminate(&child);
            assert!(child.wait().await.unwrap().success());
            reported
        })
        .await
        .expect("dashboard did not stop");
        let dir = work.path().join("report");
        let s = read(&dir, "summary.json");
        assert_eq!(s["status"], "stopped");
        assert!(s["calls"]["count"].as_u64().unwrap() > 0);
        assert_eq!(s["calls"]["failed"], 0);
        assert_eq!(s["sessions"], 0);
        assert_eq!(s["in_flight"], 0);
        assert!(dir.join("index.html").is_file());
        let bird_dir = dir.join("birds/bird-01");
        let bird = read(&bird_dir, "summary.json");
        assert_eq!(bird["status"], "stopped");
        assert!(bird["calls"]["count"].as_u64().unwrap() >= reported);
        assert_eq!(bird["calls"]["failed"], 0);
        assert_eq!(bird["in_flight"], 0);
        assert_eq!(bird["sessions"], 0);
        assert!(bird_dir.join("index.html").is_file());
    }

    #[tokio::test]
    async fn sigterm_drains_a_headless_operation_and_preserves_its_failure() {
        let work = tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = SAMPLE
            .replace(
                "127.0.0.1:8081",
                &listener.local_addr().unwrap().to_string(),
            )
            .replace("users = 2", "users = 1");
        fs::write(work.path().join("work.toml"), cfg).unwrap();
        let mut child = starling()
            .current_dir(work.path())
            .args(["run", "--config", "work.toml", "--report", "report"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let (peer, _) = timeout(Duration::from_secs(8), listener.accept())
            .await
            .unwrap()
            .unwrap();
        terminate(&child);
        assert!(
            timeout(Duration::from_millis(100), child.wait())
                .await
                .is_err(),
            "process exited with a handshake still in flight"
        );
        drop(peer);
        let exit = timeout(Duration::from_secs(8), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(exit.code(), Some(1));
        let s = read(&work.path().join("report"), "summary.json");
        assert_eq!(s["status"], "failed");
        assert_eq!(s["operations"]["initialize"]["failed"], 1);
        assert_eq!(s["calls"]["count"], 0);
        assert_eq!(s["in_flight"], 0);
    }
}
