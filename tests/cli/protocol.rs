use super::*;
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::json;
use std::sync::mpsc;

struct Seen {
    method: String,
    version: Option<String>,
    header: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Pages {
    Valid,
    NoHints,
    InvalidSchema,
}

// A fixed legacy revision makes the peer refuse discovery and counter-offer that revision at initialize.
async fn wire_peer(
    legacy_only: Option<&'static str>,
    pages: Pages,
) -> (String, mpsc::Receiver<Seen>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let app = Router::new().route("/mcp", post(move |headers: HeaderMap, Json(msg): Json<Value>| {
        let tx = tx.clone();
        async move {
            let method = msg["method"].as_str().unwrap();
            let version = msg["params"]["protocolVersion"].as_str()
                .or_else(|| msg["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str());
            tx.send(Seen {
                method: method.to_owned(), version: version.map(str::to_owned),
                header: headers.get("mcp-protocol-version").map(|v| v.to_str().unwrap().to_owned()),
            }).unwrap();
            let id = &msg["id"];
            let result = match method {
                "server/discover" if legacy_only.is_none() => json!({"resultType":"complete", "supportedVersions":["2026-07-28"],
                    "capabilities":{"tools":{}}, "ttlMs":0, "cacheScope":"private"}),
                "initialize" => json!({"protocolVersion": legacy_only.or(version).unwrap(), "capabilities":{"tools":{}},
                    "serverInfo":{"name":"fixture","version":"1"}}),
                "notifications/initialized" => return StatusCode::ACCEPTED.into_response(),
                "tools/list" if pages == Pages::InvalidSchema => json!({"tools":[{"name":"PRIVATE_TOOL","description":"PRIVATE_CONTENT","inputSchema":{"type":"array"}}]}),
                "tools/list" if version == Some("2026-07-28") && pages == Pages::Valid => json!({"tools":[], "resultType":"complete", "ttlMs":0, "cacheScope":"private"}),
                "tools/list" => json!({"tools":[]}),
                _ => return Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}})).into_response(),
            };
            Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, rx, server)
}

fn workload(url: &str, protocol: Option<&str>) -> String {
    let mut target = json!({"url":url});
    if let Some(version) = protocol {
        target["protocol"] = json!(version);
    }
    serde_yaml_ng::to_string(&json!({
        "run":{"name":"Revision probe","duration":"1s","seed":1}, "target":target,
        "load":{"users":1,"wait":"200ms","timeout":"3s"},
        "tasks":[{"method":"tools/list","weight":1,"assert_exists":"/tools"}]
    }))
    .unwrap()
}

#[tokio::test]
async fn pinned_revisions_drive_startup_and_subsequent_workload_requests() {
    let (url, rx, server) = wire_peer(None, Pages::Valid).await;
    for version in ["2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"] {
        let (work, s) = run_file(workload(&url, Some(version)), "case.yaml", &[], true).await;
        assert_eq!(
            read(&work.path().join("report"), "run.json")["protocol"],
            version
        );
        assert_eq!(s["protocols"], json!([version]));
        let wire: Vec<_> = rx.try_iter().collect();
        let modern = version == "2026-07-28";
        assert_eq!(
            wire[0].method,
            if modern {
                "server/discover"
            } else {
                "initialize"
            }
        );
        assert_eq!(wire[0].version.as_deref(), Some(version));
        let other_startup = if modern {
            "initialize"
        } else {
            "server/discover"
        };
        assert!(!wire.iter().any(|m| m.method == other_startup));
        let calls: Vec<_> = wire.iter().filter(|m| m.method == "tools/list").collect();
        assert!(!calls.is_empty());
        assert_eq!(calls.len() as u64, s["calls"]["count"].as_u64().unwrap());
        assert!(calls.iter().all(|m| m.header.as_deref() == Some(version)));
        if modern {
            assert!(calls.iter().all(|m| m.version.as_deref() == Some(version)));
        }
    }
    server.abort();
}

#[tokio::test]
async fn only_auto_can_accept_a_different_supported_revision() {
    let (url, rx, server) = wire_peer(Some("2025-06-18"), Pages::Valid).await;
    for (pin, first) in [
        ("2026-07-28", "server/discover"),
        ("2025-11-25", "initialize"),
    ] {
        let (_, s) = run_file(
            workload(&url, Some("auto")),
            "case.yaml",
            &["--protocol", pin],
            false,
        )
        .await;
        assert_eq!(s["run"]["protocol"], pin);
        assert_eq!(s["protocols"], json!([]));
        assert_eq!(s["calls"]["count"], 0);
        assert_eq!(
            s["operations"][first]["outcomes"]["protocol_version"], 1,
            "{s}"
        );
        assert!(s["reason"].as_str().unwrap().contains(pin));
        let wire: Vec<_> = rx.try_iter().collect();
        assert_eq!(wire[0].method, first);
        assert_eq!(wire[0].version.as_deref(), Some(pin));
        assert!(!wire.iter().any(|m| m.method == "tools/list"));
        if first == "server/discover" {
            assert!(!wire.iter().any(|m| m.method == "initialize"));
        }
    }
    let (_, s) = run_file(workload(&url, None), "case.yaml", &[], true).await;
    assert_eq!(s["run"]["protocol"], "auto");
    assert_eq!(s["protocols"], json!(["2025-06-18"]));
    let wire: Vec<_> = rx.try_iter().collect();
    assert_eq!(wire[0].method, "server/discover");
    assert_eq!(wire[1].method, "initialize");
    assert_eq!(wire[1].version.as_deref(), Some("2025-11-25"));
    assert!(
        wire.iter()
            .any(|m| m.method == "tools/list" && m.header.as_deref() == Some("2025-06-18"))
    );
    server.abort();
}

#[tokio::test]
async fn conformance_matrix_distinguishes_a_legacy_pass_from_missing_modern_fields() {
    let (url, rx, server) = wire_peer(None, Pages::NoHints).await;
    let tmp = tempdir().unwrap();
    let yaml = format!(
        "{}\nconformance:\n  revisions: ['2025-11-25','2026-07-28']\n",
        workload(&url, None)
    );
    fs::write(tmp.path().join("cases.yaml"), yaml).unwrap();
    let mut collector = starling()
        .current_dir(tmp.path())
        .args([
            "collect",
            "--expect",
            "wing",
            "--protocol",
            "2026-07-28,2025-11-25",
            "--deadline-secs",
            "15",
            "--report",
            "fleet",
        ])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut ready = BufReader::new(collector.stdout.take().unwrap()).lines();
    let home = timeout(
        Duration::from_secs(8),
        announced(&mut ready, "Check collector: "),
    )
    .await
    .unwrap();
    let campaign = announced(&mut ready, "Check campaign: ").await;
    let out = starling()
        .current_dir(tmp.path())
        .args([
            "check",
            "--config",
            "cases.yaml",
            "--report",
            "evidence",
            "--id",
            "wing",
            "--home",
            &home,
            "--campaign",
            &campaign,
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result = read(&tmp.path().join("evidence"), "conformance.json");
    assert_eq!(result["verdict"], "fail");
    assert_eq!(
        timeout(Duration::from_secs(15), collector.wait())
            .await
            .unwrap()
            .unwrap()
            .code(),
        Some(1)
    );
    let fleet = read(&tmp.path().join("fleet"), "fleet-conformance.json");
    assert_eq!(fleet["verdict"], "fail");
    assert_eq!(fleet["workers"]["wing"], result);
    let profiles = result["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0]["verdict"], "pass");
    assert_eq!(profiles[1]["negotiated"], "2026-07-28");
    assert_eq!(profiles[1]["verdict"], "fail");
    let row = |i: usize, rule: &str| {
        profiles[i]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["rule"] == rule)
            .unwrap()
            .clone()
    };
    assert_eq!(row(0, "catalog.tools")["evidence"], "catalog_page");
    assert_eq!(
        row(1, "catalog.tools")["evidence"],
        "missing_completion_fields"
    );
    assert_eq!(row(1, "catalog.prompts")["status"], "skip");
    let wire: Vec<_> = rx.try_iter().collect();
    assert_eq!(wire.iter().filter(|r| r.method == "tools/list").count(), 2);
    assert!(!wire.iter().any(|r| r.method == "tools/call"));
    server.abort();
}

#[tokio::test]
async fn unreachable_is_inconclusive_not_a_standards_violation() {
    let idle = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tmp = tempdir().unwrap();
    let mut cfg: Value = serde_yaml_ng::from_str(&workload(
        &format!("http://{}/mcp", idle.local_addr().unwrap()),
        Some("2025-06-18"),
    ))
    .unwrap();
    cfg["load"]["timeout"] = json!("100ms");
    fs::write(
        tmp.path().join("case.yaml"),
        serde_yaml_ng::to_string(&cfg).unwrap(),
    )
    .unwrap();
    let out = timeout(
        Duration::from_secs(5),
        starling()
            .current_dir(tmp.path())
            .args(["check", "--config", "case.yaml", "--report", "result"])
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let report = read(&tmp.path().join("result"), "conformance.json");
    assert_eq!(report["verdict"], "inconclusive");
    assert_eq!(report["profiles"][0]["checks"][0]["evidence"], "timeout");
}

#[tokio::test]
async fn conformance_reports_unsupported_and_does_not_run_dependent_rules() {
    let (url, _rx, server) = wire_peer(Some("2025-06-18"), Pages::Valid).await;
    let tmp = tempdir().unwrap();
    fs::write(
        tmp.path().join("case.yaml"),
        workload(&url, Some("2026-07-28")),
    )
    .unwrap();
    let out = starling()
        .current_dir(tmp.path())
        .args(["check", "--config", "case.yaml", "--report", "evidence"])
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    let result = read(&tmp.path().join("evidence"), "conformance.json");
    assert_eq!(result["verdict"], "unsupported");
    let p = &result["profiles"][0];
    assert!(p["negotiated"].is_null());
    assert!(
        p["checks"].as_array().unwrap()[1..]
            .iter()
            .all(|r| r["status"] == "not_run")
    );
    server.abort();
}

#[tokio::test]
async fn invalid_tool_schema_fails_without_exporting_catalog_or_target_data() {
    let (url, _rx, server) = wire_peer(None, Pages::InvalidSchema).await;
    let tmp = tempdir().unwrap();
    let out = starling()
        .current_dir(tmp.path())
        .args([
            "check",
            "--url",
            &url,
            "--protocol",
            "2025-11-25",
            "--report",
            "result",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let report = read(&tmp.path().join("result"), "conformance.json");
    assert_eq!(report["verdict"], "fail");
    let checks = report["profiles"][0]["checks"].as_array().unwrap();
    assert!(
        checks
            .iter()
            .any(|r| r["rule"] == "catalog.tools" && r["evidence"] == "invalid_tool_schema")
    );
    let contents = report.to_string();
    assert!(!contents.contains("PRIVATE_") && !contents.contains(&url));
    server.abort();
}
