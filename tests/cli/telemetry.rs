use super::*;
use std::net::TcpListener;

fn cli(dir: &Path) -> Command {
    let mut cmd = starling();
    cmd.current_dir(dir).kill_on_drop(true);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("OTEL_") {
            cmd.env_remove(name);
        }
    }
    cmd
}

#[tokio::test]
async fn default_logs_are_structured_and_an_ambient_collector_does_not_enable_export() {
    let work = tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    for (mode, dir) in [(None, "default"), (Some("none"), "silent")] {
        let mut cmd = cli(work.path());
        if let Some(mode) = mode {
            cmd.env("STARLING_TELEMETRY", mode);
        }
        let out = cmd
            .env("OTEL_EXPORTER_OTLP_ENDPOINT", &origin)
            .args([
                "demo",
                "--duration",
                "1s",
                "--wait",
                "100ms",
                "--report",
                dir,
            ])
            .output()
            .await
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8(out.stdout)
                .unwrap()
                .contains("Demo fixture:")
        );
        let rows: Vec<Value> = String::from_utf8(out.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        if mode.is_none() {
            assert_eq!(
                rows.iter()
                    .map(|r| r["event"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                [
                    "process_started",
                    "run_started",
                    "run_finished",
                    "process_finished"
                ]
            );
            let end = &rows[2];
            assert_eq!(
                end["requests"],
                read(&work.path().join(dir), "summary.json")["calls"]["count"]
            );
            assert_eq!(end["role"], "runner");
            assert_eq!(end["level"], "info");
            assert_eq!(end["service"], "starling");
            assert_eq!(end["status"], "completed");
            assert!(end["timestampMs"].as_u64().unwrap() > 0);
        } else {
            assert!(rows.is_empty());
        }
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn invalid_export_configuration_is_rejected_before_the_run() {
    let work = tempdir().unwrap();
    for endpoint in [
        "http://collector.invalid",
        "https://user:DO_NOT_LOG@localhost",
        "https://localhost/prefix/..",
        "http://127.0.0.1?key=DO_NOT_LOG",
    ] {
        let out = cli(work.path())
            .env("STARLING_TELEMETRY", "otlp")
            .env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
            .args(["demo", "--report", "unused"])
            .output()
            .await
            .unwrap();
        assert!(!out.status.success());
        assert!(!work.path().join("unused").exists());
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(!err.contains("DO_NOT_LOG"));
        assert!(
            err.contains(if cfg!(feature = "otlp") {
                "OTLP collector"
            } else {
                "without --features otlp"
            }),
            "{err}"
        );
    }
}

#[cfg(feature = "otlp")]
mod wire {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        extract::State,
        http::{HeaderMap, Uri},
        routing::post,
    };
    use opentelemetry_proto::tonic::{
        collector::{
            logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
            trace::v1::ExportTraceServiceRequest,
        },
        common::v1::any_value,
        metrics::v1::{metric::Data, number_data_point},
        resource::v1::Resource,
    };
    use prost::Message;
    use tokio::sync::mpsc;

    fn identity(resource: Option<&Resource>) -> String {
        let attrs = &resource.unwrap().attributes;
        let get = |key| match attrs
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| v.value.as_ref())
        {
            Some(any_value::Value::StringValue(value)) => value.clone(),
            _ => panic!("missing resource attribute {key}"),
        };
        assert_eq!(get("service.name"), "starling");
        assert_eq!(get("starling.role"), "runner");
        get("service.instance.id")
    }

    #[tokio::test]
    async fn failed_workload_flushes_correlated_signals_and_only_scheduled_samples() {
        let (mut fixture, target) = fixture().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, Bytes)>();
        let server = Router::new()
            .route(
                "/v1/{kind}",
                post(
                    |State(tx): State<mpsc::UnboundedSender<(String, Bytes)>>,
                     uri: Uri,
                     headers: HeaderMap,
                     body: Bytes| async move {
                        assert_eq!(headers["content-type"], "application/x-protobuf");
                        tx.send((uri.path().into(), body)).unwrap();
                        ([("content-type", "application/x-protobuf")], Bytes::new())
                    },
                ),
            )
            .with_state(tx);
        let peer = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let work = tempdir().unwrap();
        let yaml = format!(
            "run: {{ name: PRIVATE_RUN, duration: 1s, seed: 1 }}\ntarget: {{ url: '{target}' }}\n\
            load: {{ users: 1, wait: 150ms, timeout: 2s }}\n\
            tasks: [{{ name: PRIVATE_CASE, tool: echo, weight: 1, arguments: {{ text: PRIVATE_QUESTION }}, assert_exists: /missing }}]\n"
        );
        fs::write(work.path().join("work.yaml"), yaml).unwrap();
        let out = timeout(
            Duration::from_secs(15),
            cli(work.path())
                .env("STARLING_TELEMETRY", "stderr,otlp")
                .env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
                .env("OTEL_METRIC_EXPORT_INTERVAL", "60000")
                .env("OTEL_RESOURCE_ATTRIBUTES", "unused=PRIVATE_RESOURCE")
                .env(
                    "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
                    "http://127.0.0.1:1/wrong",
                )
                .args(["run", "--config", "work.yaml", "--report", "report"])
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.contains("one or more workload requests failed"),
            "{stderr}"
        );
        let local = stderr
            .lines()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .find(|r| r["event"] == "run_finished")
            .unwrap();
        let summary = read(&work.path().join("report"), "summary.json");
        let count = summary["calls"]["count"].as_u64().unwrap();
        assert!(count > 0);
        assert_eq!(summary["calls"]["failed"], count);
        let (mut logs, mut spans, mut metrics) = (Vec::new(), Vec::new(), Vec::new());
        let mut ids = std::collections::BTreeSet::new();
        while let Ok((path, bytes)) = rx.try_recv() {
            assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE_"));
            match path.as_str() {
                "/v1/logs" => {
                    for r in ExportLogsServiceRequest::decode(bytes)
                        .unwrap()
                        .resource_logs
                    {
                        ids.insert(identity(r.resource.as_ref()));
                        logs.extend(r.scope_logs.into_iter().flat_map(|s| s.log_records));
                    }
                }
                "/v1/traces" => {
                    for r in ExportTraceServiceRequest::decode(bytes)
                        .unwrap()
                        .resource_spans
                    {
                        ids.insert(identity(r.resource.as_ref()));
                        spans.extend(r.scope_spans.into_iter().flat_map(|s| s.spans));
                    }
                }
                "/v1/metrics" => {
                    for r in ExportMetricsServiceRequest::decode(bytes)
                        .unwrap()
                        .resource_metrics
                    {
                        ids.insert(identity(r.resource.as_ref()));
                        metrics.extend(r.scope_metrics.into_iter().flat_map(|s| s.metrics));
                    }
                }
                _ => panic!("unexpected signal: {path}"),
            }
        }
        assert_eq!(ids.len(), 1);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.name, "starling.run");
        assert_eq!(span.status.as_ref().unwrap().code, 2);
        let end = logs
            .iter()
            .find(|r| r.event_name == "run_finished")
            .unwrap();
        assert_eq!(
            (&end.trace_id, &end.span_id),
            (&span.trace_id, &span.span_id)
        );
        assert_eq!(end.severity_number, 17);
        assert_eq!(
            local["traceId"],
            span.trace_id
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let find = |name| {
            metrics
                .iter()
                .find(|m| m.name == name)
                .unwrap()
                .data
                .as_ref()
                .unwrap()
        };
        let Data::Sum(sum) = find("starling.workload.requests") else {
            panic!("expected request counter")
        };
        assert!(sum.is_monotonic);
        assert_eq!(sum.data_points.len(), 1);
        let point = &sum.data_points[0];
        assert_eq!(
            point.value,
            Some(number_data_point::Value::AsInt(count as i64))
        );
        assert!(point.attributes.iter().any(|a| a.key == "starling.outcome"
            && a.value.as_ref().unwrap().value
                == Some(any_value::Value::StringValue("assertion".into()))));
        let Data::Histogram(hist) = find("starling.workload.duration") else {
            panic!("expected duration histogram")
        };
        let distribution = &hist.data_points[0];
        assert_eq!(distribution.count, count);
        assert_eq!(distribution.bucket_counts.iter().sum::<u64>(), count);
        assert!(!distribution.explicit_bounds.is_empty());
        assert!(!stderr.contains("PRIVATE_"));
        peer.abort();
        fixture.kill().await.unwrap();
    }

    #[tokio::test]
    async fn collector_rejection_does_not_change_the_saved_workload_result() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let receiver = Router::new().route(
            "/v1/{signal}",
            post(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "PRIVATE_RECEIVER_DETAIL",
                )
            }),
        );
        let peer = tokio::spawn(async move { axum::serve(listener, receiver).await.unwrap() });
        let work = tempdir().unwrap();
        let out = timeout(
            Duration::from_secs(12),
            cli(work.path())
                .env("STARLING_TELEMETRY", "otlp")
                .env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
                .args([
                    "demo",
                    "--duration",
                    "1s",
                    "--wait",
                    "100ms",
                    "--report",
                    "local",
                ])
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let error = String::from_utf8(out.stderr).unwrap();
        assert!(error.contains("OTLP"), "{error}");
        assert!(!error.contains("PRIVATE_RECEIVER_DETAIL"));
        let report = read(&work.path().join("local"), "summary.json");
        assert_eq!(report["status"], "completed");
        assert!(report["calls"]["count"].as_u64().unwrap() > 0);
        assert_eq!(report["calls"]["failed"], 0);
        peer.abort();
    }
}
