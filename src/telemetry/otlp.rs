use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, ensure};
use opentelemetry::{
    Context, KeyValue,
    logs::{LogRecord, Logger, LoggerProvider, Severity},
    metrics::{Counter, Histogram, MeterProvider},
    trace::{SpanId, Status as SpanStatus, TraceContextExt, TraceId, Tracer, TracerProvider},
};
use opentelemetry_otlp::{
    LogExporter, MetricExporter, Protocol, RetryPolicy, SpanExporter, WithExportConfig,
    WithHttpConfig,
};
use opentelemetry_sdk::{
    Resource,
    logs::{SdkLogger, SdkLoggerProvider},
    metrics::SdkMeterProvider,
    trace::{SdkTracer, SdkTracerProvider},
};
use reqwest::{blocking::Client, redirect::Policy};
use url::{Host, Url};

use super::{Level, RunTrace, Totals};
use crate::{config::Method, metrics::Outcome, runner::Status};

pub(super) struct Export {
    logs: SdkLoggerProvider,
    logger: SdkLogger,
    meters: SdkMeterProvider,
    traces: SdkTracerProvider,
    tracer: SdkTracer,
    requests: Counter<u64>,
    latency: Histogram<f64>,
}

fn collector(value: &str) -> Result<Url> {
    const INVALID: &str = "OTLP collector must be an HTTPS origin or numeric loopback HTTP origin, without credentials, path, query or fragment";
    let url = Url::parse(value).ok();
    let url = url
        .filter(|url| {
            let loopback = matches!(url.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
                || matches!(url.host(), Some(Host::Ipv6(ip)) if ip.is_loopback());
            url.has_host()
                && (url.scheme() == "https" || (url.scheme() == "http" && loopback))
                && value.split_once("://").is_some_and(|(_, rest)| {
                    !rest.trim_end_matches('/').contains('/') && !rest.ends_with("//")
                })
                && !value.contains(['@', '?', '#', '\\'])
                && !value.chars().any(|c| c.is_whitespace() || c.is_control())
        })
        .context(INVALID)?;
    Ok(url)
}

impl Export {
    pub(super) fn open(endpoint: &str, role: &'static str, bird: Option<String>) -> Result<Self> {
        let origin = collector(endpoint)?;
        let timeout = Duration::from_secs(3);
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .timeout(timeout)
            .build()
            .map_err(|_| anyhow::anyhow!("could not initialize OTLP HTTP client"))?;
        fn signal<B: WithExportConfig + WithHttpConfig>(
            builder: B,
            client: &Client,
            url: Url,
            limit: Duration,
        ) -> B {
            builder
                .with_http_client(client.clone())
                .with_endpoint(url.as_str())
                .with_protocol(Protocol::HttpBinary)
                .with_timeout(limit)
                .with_retry_policy(RetryPolicy::disabled())
        }
        let logs = signal(
            LogExporter::builder().with_http(),
            &client,
            origin.join("/v1/logs")?,
            timeout,
        )
        .build()
        .map_err(|_| anyhow::anyhow!("invalid OTLP log exporter configuration"))?;
        let spans = signal(
            SpanExporter::builder().with_http(),
            &client,
            origin.join("/v1/traces")?,
            timeout,
        )
        .build()
        .map_err(|_| anyhow::anyhow!("invalid OTLP trace exporter configuration"))?;
        let metrics = signal(
            MetricExporter::builder().with_http(),
            &client,
            origin.join("/v1/metrics")?,
            timeout,
        )
        .build()
        .map_err(|_| anyhow::anyhow!("invalid OTLP metric exporter configuration"))?;
        let mut attributes = vec![
            KeyValue::new("starling.role", role),
            KeyValue::new(
                "service.instance.id",
                format!("{:032x}", fastrand::u128(..)),
            ),
        ];
        if let Some(id) = bird {
            attributes.push(KeyValue::new("starling.bird.id", id));
        }
        let resource = Resource::builder_empty()
            .with_service_name("starling")
            .with_attributes(attributes)
            .build();
        let logs = SdkLoggerProvider::builder()
            .with_resource(resource.clone())
            .with_batch_exporter(logs)
            .build();
        let traces = SdkTracerProvider::builder()
            .with_resource(resource.clone())
            .with_batch_exporter(spans)
            .build();
        let meters = SdkMeterProvider::builder()
            .with_resource(resource)
            .with_periodic_exporter(metrics)
            .build();
        let meter = meters.meter("starling");
        Ok(Self {
            logger: logs.logger("starling"),
            tracer: traces.tracer("starling"),
            logs,
            traces,
            meters,
            requests: meter.u64_counter("starling.workload.requests").build(),
            latency: meter
                .f64_histogram("starling.workload.duration")
                .with_unit("ms")
                .with_boundaries(vec![
                    1., 5., 10., 25., 50., 100., 250., 500., 1000., 2500., 5000., 10000., 15000.,
                ])
                .build(),
        })
    }

    pub(super) fn start_run(&self) -> Context {
        Context::new().with_span(self.tracer.start("starling.run"))
    }

    pub(super) fn log(
        &self,
        now: SystemTime,
        level: Level,
        event: &'static str,
        totals: Option<Totals>,
        trace: &RunTrace,
    ) -> Option<(TraceId, SpanId)> {
        let mut row = self.logger.create_log_record();
        let severity = match level {
            Level::Info => Severity::Info,
            Level::Warn => Severity::Warn,
            Level::Error => Severity::Error,
        };
        row.set_timestamp(now);
        row.set_severity_number(severity);
        row.set_severity_text(severity.name());
        row.set_event_name(event);
        row.set_body(event.into());
        if let Some(totals) = totals {
            row.add_attribute("starling.status", totals.status);
            row.add_attribute("starling.requests", totals.requests as i64);
            row.add_attribute("starling.failed", totals.failed as i64);
        }
        let ids = trace.context.as_ref().map(|ctx| {
            let span = ctx.span();
            let sc = span.span_context();
            row.set_trace_context(sc.trace_id(), sc.span_id(), Some(sc.trace_flags()));
            (sc.trace_id(), sc.span_id())
        });
        self.logger.emit(row);
        ids
    }

    pub(super) fn request(&self, method: Method, outcome: Outcome, latency: Duration) {
        let method = match method {
            Method::CallTool => "tools/call",
            Method::ListTools => "tools/list",
            Method::GetPrompt => "prompts/get",
            Method::ListPrompts => "prompts/list",
        };
        let dims = [
            KeyValue::new("mcp.method.name", method),
            KeyValue::new("starling.outcome", outcome.label()),
        ];
        self.requests.add(1, &dims);
        self.latency.record(latency.as_secs_f64() * 1000., &dims);
    }

    pub(super) fn close(&self) -> Result<()> {
        let limit = Duration::from_secs(3);
        let results = [
            ("logs", self.logs.shutdown_with_timeout(limit)),
            ("metrics", self.meters.shutdown_with_timeout(limit)),
            ("traces", self.traces.shutdown_with_timeout(limit)),
        ];
        let failed: Vec<_> = results
            .iter()
            .filter(|(_, result)| result.is_err())
            .map(|(name, _)| *name)
            .collect();
        ensure!(
            failed.is_empty(),
            "OTLP shutdown failed for {}",
            failed.join(", ")
        );
        Ok(())
    }
}

impl RunTrace {
    pub(super) fn finish(self, status: Status) {
        if let Some(ctx) = self.context {
            let span = ctx.span();
            span.set_attribute(KeyValue::new("starling.status", status.label()));
            if status == Status::Failed {
                span.set_status(SpanStatus::error("run failed"));
            }
            span.end();
        }
    }
}
