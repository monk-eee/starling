use std::{future::IntoFuture, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Request, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use tokio::{net::TcpListener, sync::watch, time::timeout};
use tokio_util::sync::CancellationToken;

use crate::{
    config::StartOptions,
    metrics::Point,
    report,
    runner::{Frame, Overview, Run},
};

#[derive(Clone)]
struct Slot {
    run: Arc<Run>,
    dir: PathBuf,
}

type Controller = watch::Sender<Slot>;

pub async fn serve(
    listener: TcpListener,
    run: Arc<Run>,
    dir: PathBuf,
    signal: impl Future<Output = Result<()>>,
) -> Result<(PathBuf, Arc<Frame>)> {
    let address = listener.local_addr()?;
    let (controller, mut runs) = watch::channel(Slot { run, dir });
    let app = Router::new()
        .route("/", get(|| async { Html(report::DASHBOARD) }))
        .route(
            "/api/v1/health",
            get(|| async { Json(serde_json::json!({"status":"ok"})) }),
        )
        .route("/api/v1/runs/local", get(snapshot))
        .route("/api/v1/runs/local/start", post(start))
        .route("/api/v1/runs/local/stop", post(stop))
        .route("/api/v1/runs/local/next", post(next_run))
        .route("/api/v1/runs/local/live", get(upgrade))
        .route("/api/v1/runs/local/export", get(export))
        .with_state(controller)
        .layer(DefaultBodyLimit::max(2048))
        .layer(middleware::from_fn_with_state(address, local_requests));
    let shutdown = CancellationToken::new();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.clone().cancelled_owned())
        .into_future();
    tokio::pin!(server);
    let lifecycle = async {
        tokio::pin!(signal);
        loop {
            let slot = runs.borrow_and_update().clone();
            let execute = slot.run.clone().execute(slot.dir.clone());
            tokio::pin!(execute);
            let frame = tokio::select! {
                result = &mut execute => result?,
                event = &mut signal => {
                    event?; slot.run.stop();
                    return Ok((slot.dir, execute.await?));
                }
            };
            println!(
                "Report: {}. Dashboard remains open; Ctrl+C to exit.",
                slot.dir.join("index.html").display()
            );
            tokio::select! {
                biased;
                changed = runs.changed() => changed?,
                event = &mut signal => { event?; return Ok((slot.dir, frame)); }
            }
        }
    };
    tokio::select! {
        result = &mut server => { result?; bail!("dashboard stopped unexpectedly"); }
        result = lifecycle => { shutdown.cancel(); server.await?; result }
    }
}

async fn local_requests(State(addr): State<SocketAddr>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let host_ok = host == addr.to_string()
        || host == format!("localhost:{}", addr.port())
        || (addr.port() == 80
            && (host == "localhost" || host == addr.to_string().trim_end_matches(":80")));
    let foreign_origin = headers
        .get(header::ORIGIN)
        .is_some_and(|v| v.as_bytes() != format!("http://{host}").as_bytes());
    if !host_ok
        || foreign_origin
        || (*req.method() == Method::POST
            && headers.get("x-starling-control").is_none_or(|v| v != "1"))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(req).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    response
}

async fn snapshot(State(controller): State<Controller>) -> impl IntoResponse {
    Json((*controller.borrow().run.snapshot()).clone())
}

async fn start(State(controller): State<Controller>, body: Bytes) -> Response {
    let options = if body.is_empty() {
        StartOptions::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    "Expected numeric load settings in a JSON object.",
                )
                    .into_response();
            }
        }
    };
    match controller.borrow().run.start(options) {
        Ok(true) => StatusCode::ACCEPTED.into_response(),
        Ok(false) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
    }
}

async fn stop(State(controller): State<Controller>) -> StatusCode {
    if controller.borrow().run.stop() {
        StatusCode::ACCEPTED
    } else {
        StatusCode::CONFLICT
    }
}

async fn next_run(State(controller): State<Controller>) -> Response {
    let mut status = StatusCode::CONFLICT;
    controller.send_if_modified(|slot| {
        if !slot.run.snapshot().view.status.terminal() {
            return false;
        }
        match report::reserve_in(slot.dir.parent().unwrap_or(&slot.dir)) {
            Ok(dir) => {
                *slot = Slot {
                    run: Run::new(slot.run.config().clone()),
                    dir,
                };
                status = StatusCode::ACCEPTED;
                true
            }
            Err(e) => {
                eprintln!("Could not reserve the next report: {e}");
                status = StatusCode::INTERNAL_SERVER_ERROR;
                false
            }
        }
    });
    status.into_response()
}

async fn export(State(controller): State<Controller>) -> Response {
    let frame = controller.borrow().run.snapshot();
    if !frame.view.status.terminal() {
        return StatusCode::CONFLICT.into_response();
    }
    match report::html(&frame) {
        Ok(page) => (
            [(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"starling-report.html\"",
            )],
            Html(page),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn upgrade(State(controller): State<Controller>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(1024)
        .max_frame_size(1024)
        .on_upgrade(move |socket| live(socket, controller.subscribe()))
}

#[derive(Serialize)]
struct MessageBody<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(flatten)]
    overview: &'a Overview,
    series: &'a [Point],
}

async fn live(mut socket: WebSocket, mut runs: watch::Receiver<Slot>) {
    let mut feed = runs.borrow_and_update().run.subscribe();
    let mut previous = None;
    loop {
        let frame = feed.borrow_and_update().clone();
        let sequence = frame.view.sequence;
        // A slow reader skips publications rather than slowing down the runner.
        let (kind, from) = match previous {
            Some((seq, len)) if seq + 1 == sequence => ("tick", len),
            _ => ("snapshot", 0),
        };
        let body = MessageBody {
            kind,
            overview: &frame.view,
            series: &frame.series[from..],
        };
        let text = serde_json::to_string(&body).expect("serializable frame");
        if !matches!(
            timeout(
                Duration::from_secs(5),
                socket.send(Message::Text(text.into()))
            )
            .await,
            Ok(Ok(()))
        ) {
            return;
        }
        previous = Some((sequence, frame.series.len()));
        loop {
            tokio::select! {
                biased;
                changed = runs.changed() => {
                    if changed.is_err() { return; }
                    feed = runs.borrow_and_update().run.subscribe();
                    previous = None;
                    break;
                }
                changed = feed.changed() => { if changed.is_err() { return; } break; }
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Text(s))) if s == "snapshot" => { previous = None; break; }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => (),
                    _ => return,
                }
            }
        }
    }
}
