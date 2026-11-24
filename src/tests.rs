use std::{future::Future, path::PathBuf, sync::Once, time::Duration};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

use crate::{config::Config, dashboard, fixture, report, runner::Run, telemetry::Logging};

pub(super) fn install_test_tls() {
    static TLS: Once = Once::new();
    TLS.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("test process TLS provider")
    });
}

struct Controller {
    http: Client,
    url: String,
    report: PathBuf,
}

impl Controller {
    fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(format!("{}/api/v1/{path}", self.url))
    }

    fn post(&self, action: &str) -> RequestBuilder {
        self.http
            .post(format!("{}/api/v1/runs/local/{action}", self.url))
            .header("x-starling-control", "1")
    }

    async fn snapshot(&self) -> Result<Value> {
        json_body(self.get("runs/local").send().await?).await
    }
}

async fn json_body(response: Response) -> Result<Value> {
    Ok(serde_json::from_slice(
        &response.error_for_status()?.bytes().await?,
    )?)
}

async fn with_controller<F: Future<Output = Result<()>>>(
    exercise: impl FnOnce(Controller) -> F,
) -> Result<()> {
    install_test_tls();
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let target_addr = target.local_addr()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let tmp = tempfile::tempdir()?;
    let dir = report::reserve(Some(tmp.path().join("first")))?;
    let controller = Controller {
        http: Client::builder().no_proxy().tls_certs_only([]).build()?,
        url: format!("http://{}", listener.local_addr()?),
        report: dir.clone(),
    };
    let logging = Logging::open("controller", None, false).await?;
    let run = Run::new(
        Config::demo(target_addr, Default::default())?,
        logging.events.clone(),
    );
    let stop = CancellationToken::new();
    timeout(
        Duration::from_secs(30),
        fixture::alongside(target, async {
            let server = dashboard::serve(
                listener,
                run,
                dir,
                Some(target_addr),
                ([127, 0, 0, 1], 0).into(),
                0,
                async {
                    stop.cancelled().await;
                    Ok(())
                },
            );
            let client = async {
                let result = exercise(controller).await;
                stop.cancel();
                result
            };
            tokio::try_join!(server, client)?;
            Ok(())
        }),
    )
    .await
    .context("controller lifecycle timed out")??;
    logging.shutdown().await
}

#[tokio::test]
async fn dashboard_checks_do_not_dispatch_work_or_replace_the_active_report() -> Result<()> {
    with_controller(|c| async move {
        let template = c.get("workload/example").send().await?.error_for_status()?.text().await?;
        let mut cfg: Value = serde_yaml_ng::from_str(&template)?;
        let held = TcpListener::bind("127.0.0.1:0").await?;
        cfg["target"]["endpoints"] = json!({"waiting":format!("http://{}/mcp", held.local_addr()?)});
        cfg["run"]["name"] = json!("Replacement");
        cfg["run"]["mode"] = json!("once");
        cfg["load"]["timeout"] = json!("15s");
        cfg["tasks"] = json!([{"method":"prompts/get","prompt":"ask","arguments":{"question":"PRIVATE_CASE"},
                              "weight":1,"assert_exists":"/messages/0/content/text"}]);
        cfg["report"] = json!({"formats":["json"]});

        assert_eq!(c.get("runs/local/export").send().await?.status(), StatusCode::CONFLICT);
        assert_eq!(c.post("workload").body("run: [").send().await?.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(c.snapshot().await?["run"]["name"], "Starling demo");
        let receipt = json_body(c.post("workload").body(serde_yaml_ng::to_string(&cfg)?).send().await?).await?;
        assert_eq!(receipt["generation"], 2);

        for (body, status) in [
            ("not-json", StatusCode::BAD_REQUEST),
            (r#"{"endpoint":"absent"}"#, StatusCode::UNPROCESSABLE_ENTITY),
            (r#"{"all_protocols":true,"protocol":"auto"}"#, StatusCode::UNPROCESSABLE_ENTITY),
        ] {
            assert_eq!(c.post("check").body(body).send().await?.status(), status);
        }
        let check = c.post("check").body(r#"{"endpoint":"waiting","protocol":"2025-06-18"}"#);
        let blocked = async {
            let (peer, _) = held.accept().await?;
            for action in ["start", "check"] {
                assert_eq!(c.post(action).body("{}").send().await?.status(), StatusCode::CONFLICT);
            }
            drop(peer);
            Ok::<_, anyhow::Error>(())
        };
        let (inconclusive, ()) = tokio::try_join!(async { json_body(check.send().await?).await }, blocked)?;
        assert_eq!(inconclusive["verdict"], "inconclusive");
        let valid = json_body(c.post("check").body(r#"{"protocol":"2025-06-18"}"#).send().await?).await?;
        assert_eq!(valid["profiles"][0]["negotiated"], "2025-06-18");
        assert_eq!(valid["verdict"], "pass");
        let preview = c.snapshot().await?;
        assert_eq!(preview["run"]["name"], "Replacement");
        assert_eq!(preview["status"], "ready");
        assert_eq!(preview["calls"]["count"], 0);

        assert_eq!(c.post("start").body(r#"{"users":0}"#).send().await?.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(c.post("start").body(r#"{"users":1}"#).send().await?.status(), StatusCode::ACCEPTED);
        let saved = loop {
            let frame = c.snapshot().await?;
            match frame["status"].as_str() {
                Some("completed") => break frame,
                Some("running" | "stopping") => sleep(Duration::from_millis(25)).await,
                _ => anyhow::bail!("unexpected run state: {}", frame["status"]),
            }
        };
        assert_eq!(saved["calls"]["count"], 1);
        assert_eq!(saved["assertions"][0]["status"], "passed");
        assert_eq!(c.post("check").body("{}").send().await?.status(), StatusCode::CONFLICT);
        let original = std::fs::read(c.report.join("summary.json"))?;
        let download = c.get("runs/local/export?file=summary.json").send().await?.error_for_status()?;
        assert_eq!(download.headers()["content-type"], "application/json");
        assert_eq!(download.bytes().await?.as_ref(), original.as_slice());
        let html = c.get("runs/local/export").send().await?.error_for_status()?;
        assert_eq!(html.headers()["content-disposition"], "attachment; filename=\"starling-report.html\"");
        assert!(!c.report.join("index.html").exists());
        let html = html.text().await?;
        assert!(html.contains("\"name\":\"Replacement\"") && !html.contains("PRIVATE_CASE"));
        assert_eq!(c.get("runs/local/export?file=workload.yaml").send().await?.status(), StatusCode::BAD_REQUEST);
        let next = json_body(c.post("next").send().await?).await?;
        assert_eq!(next["generation"], 3);
        assert_eq!(c.snapshot().await?["calls"]["count"], 0);
        assert_eq!(std::fs::read(c.report.join("summary.json"))?, original);
        Ok(())
    }).await
}

async fn message(ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> Result<Value> {
    let data = ws.next().await.context("socket closed before snapshot")??;
    Ok(serde_json::from_str(data.to_text()?)?)
}

#[tokio::test]
async fn dashboard_resync_returns_full_history_then_resets_for_the_next_run() -> Result<()> {
    with_controller(|c| async move {
        let base = c.url.replacen("http:", "ws:", 1);
        let (mut flock, _) = connect_async(format!("{base}/api/v1/flock/live")).await?;
        let roster = message(&mut flock).await?;
        assert_eq!(roster["birds"], json!([]));
        assert_eq!(
            roster["home"],
            json_body(c.get("flock").send().await?).await?["home"]
        );
        flock.close(None).await?;

        let (mut ws, _) = connect_async(format!("{base}/api/v1/runs/local/live")).await?;
        let initial = message(&mut ws).await?;
        assert_eq!(initial["type"], "snapshot");
        assert_eq!(initial["status"], "ready");
        assert_eq!(
            c.post("start")
                .body(r#"{"wait_ms":100}"#)
                .send()
                .await?
                .status(),
            StatusCode::ACCEPTED
        );
        let tick = loop {
            let m = message(&mut ws).await?;
            if m["type"] == "tick" && m["series"][0]["second"].as_u64().is_some_and(|s| s > 0) {
                break m;
            }
        };
        ws.send(Message::Text("snapshot".into())).await?;
        let full = loop {
            let m = message(&mut ws).await?;
            if m["type"] == "snapshot" {
                break m;
            }
        };
        assert_eq!(full["generation"], initial["generation"]);
        assert!(full["sequence"].as_u64().unwrap() >= tick["sequence"].as_u64().unwrap());
        assert_eq!(full["series"][0]["second"], 0);
        assert!(
            full["series"].as_array().unwrap().len() > tick["series"].as_array().unwrap().len()
        );
        assert_eq!(c.post("stop").send().await?.status(), StatusCode::ACCEPTED);
        while message(&mut ws).await?["status"] != "stopped" {}
        ws.send(Message::Text("snapshot".into())).await?;
        let drained = message(&mut ws).await?;
        assert_eq!(drained["type"], "snapshot");
        let total: u64 = drained["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["calls"].as_u64().unwrap())
            .sum();
        assert!(total > 0);
        assert_eq!(drained["calls"]["count"], total);
        let receipt = json_body(c.post("next").send().await?).await?;
        let fresh = message(&mut ws).await?;
        assert_eq!(fresh["type"], "snapshot");
        assert_eq!(fresh["generation"], receipt["generation"]);
        assert_ne!(fresh["generation"], initial["generation"]);
        assert_eq!(fresh["calls"]["count"], 0);
        assert_eq!(fresh["series"], json!([]));
        ws.close(None).await?;
        Ok(())
    })
    .await
}
