use std::{
    collections::BTreeMap,
    env, fs,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
    routing::get,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;
use tonic::{
    Request, Response as RpcResponse, Status,
    transport::{Certificate, Identity, Server, ServerTlsConfig},
};

pub mod rpc {
    tonic::include_proto!("starling.flock.v1");
}
use rpc::{
    Heartbeat, Receipt,
    telemetry_server::{Telemetry, TelemetryServer},
};

pub const STALE: Duration = Duration::from_secs(5);
const RETAIN: Duration = Duration::from_secs(300);
const MAX_BIRDS: usize = 64;

pub fn bird_id(value: &str) -> Result<String, String> {
    if (1..=32).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        Ok(value.into())
    } else {
        Err("bird ID must use 1-32 letters, digits, hyphens or underscores".into())
    }
}

pub struct Credentials {
    pub ca: Certificate,
    pub identity: Identity,
}

impl Credentials {
    pub fn from_env() -> Result<Option<Self>> {
        match (
            env::var_os("STARLING_FLOCK_CA"),
            env::var_os("STARLING_FLOCK_CERT"),
            env::var_os("STARLING_FLOCK_KEY"),
        ) {
            (None, None, None) => Ok(None),
            (Some(ca), Some(cert), Some(key)) => Ok(Some(Self {
                ca: Certificate::from_pem(fs::read(ca).context("reading flock CA")?),
                identity: Identity::from_pem(
                    fs::read(cert).context("reading flock certificate")?,
                    fs::read(key).context("reading flock private key")?,
                ),
            })),
            _ => anyhow::bail!(
                "set STARLING_FLOCK_CA, STARLING_FLOCK_CERT and STARLING_FLOCK_KEY together"
            ),
        }
    }
}

struct Seen {
    pulse: Heartbeat,
    at: Instant,
}

#[derive(Clone, Default)]
pub struct Flock(Arc<Mutex<BTreeMap<String, Seen>>>);

#[derive(Serialize)]
struct BirdView {
    id: String,
    pid: u32,
    state: String,
    age_ms: u64,
    stale: bool,
    requests: u64,
    failed: u64,
    sessions: u32,
    in_flight: u32,
    p95_ms: Option<f64>,
}

impl Flock {
    fn record(&self, pulse: Heartbeat, now: Instant) -> Result<(), Status> {
        bird_id(&pulse.bird_id).map_err(Status::invalid_argument)?;
        if pulse.incarnation == 0
            || pulse.sequence == 0
            || pulse.pid == 0
            || !matches!(
                pulse.state.as_str(),
                "idle" | "ready" | "running" | "stopping" | "completed" | "stopped" | "failed"
            )
            || pulse.failed > pulse.requests
            || pulse.requests > (1 << 53) - 1
            || pulse.p95_ms.is_some_and(|p| !p.is_finite() || p < 0.0)
        {
            return Err(Status::invalid_argument("invalid heartbeat"));
        }
        let mut birds = self.0.lock().unwrap();
        birds.retain(|_, seen| now.duration_since(seen.at) < RETAIN);
        if let Some(seen) = birds.get(&pulse.bird_id) {
            if seen.pulse.incarnation == pulse.incarnation {
                // Retransmission must not add requests or make an old observation look fresh.
                if pulse.sequence <= seen.pulse.sequence {
                    return Ok(());
                }
                if pulse.requests < seen.pulse.requests || pulse.failed < seen.pulse.failed {
                    return Err(Status::invalid_argument("counters went backwards"));
                }
            } else if now.duration_since(seen.at) < STALE {
                return Err(Status::already_exists("another live bird has this ID"));
            }
        } else if birds.len() >= MAX_BIRDS {
            return Err(Status::resource_exhausted("flock is full"));
        }
        birds.insert(pulse.bird_id.clone(), Seen { pulse, at: now });
        Ok(())
    }

    fn view(&self, now: Instant) -> Vec<BirdView> {
        self.0
            .lock()
            .unwrap()
            .values()
            .filter_map(|s| {
                let age = now.duration_since(s.at);
                let p = &s.pulse;
                (age < RETAIN).then(|| BirdView {
                    id: p.bird_id.clone(),
                    pid: p.pid,
                    state: p.state.clone(),
                    age_ms: age.as_millis() as u64,
                    stale: age >= STALE,
                    requests: p.requests,
                    failed: p.failed,
                    sessions: p.sessions,
                    in_flight: p.in_flight,
                    p95_ms: p.p95_ms,
                })
            })
            .collect()
    }
}

#[tonic::async_trait]
impl Telemetry for Flock {
    async fn check_in(&self, request: Request<Heartbeat>) -> Result<RpcResponse<Receipt>, Status> {
        self.record(request.into_inner(), Instant::now())?;
        Ok(RpcResponse::new(Receipt {}))
    }
}

pub struct Hub {
    listener: TcpListener,
    tls: Option<ServerTlsConfig>,
    pub flock: Flock,
    pub endpoint: String,
}

impl Hub {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let tls = Credentials::from_env()?.map(|c| {
            ServerTlsConfig::new()
                .identity(c.identity)
                .client_ca_root(c.ca)
        });
        ensure!(
            addr.ip().is_loopback() || tls.is_some(),
            "a non-loopback bird listener requires mutual TLS"
        );
        let listener = TcpListener::bind(addr)
            .await
            .context("binding bird telemetry listener")?;
        let scheme = if tls.is_some() { "https" } else { "http" };
        let endpoint = format!("{scheme}://{}", listener.local_addr()?);
        Ok(Self {
            listener,
            tls,
            flock: Flock::default(),
            endpoint,
        })
    }

    pub async fn serve(self, stop: CancellationToken) -> Result<()> {
        let mut server = Server::builder()
            .max_concurrent_streams(16)
            .concurrency_limit_per_connection(4)
            .timeout(Duration::from_secs(3));
        if let Some(tls) = self.tls {
            server = server.tls_config(tls)?;
        }
        server
            .add_service(TelemetryServer::new(self.flock).max_decoding_message_size(2048))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(self.listener),
                stop.cancelled_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(Clone)]
struct Feed {
    flock: Flock,
    stop: CancellationToken,
    home: String,
}

#[derive(Serialize)]
struct Roster {
    home: String,
    birds: Vec<BirdView>,
}

impl Feed {
    fn snapshot(&self) -> Roster {
        Roster {
            home: self.home.clone(),
            birds: self.flock.view(Instant::now()),
        }
    }
}

pub fn routes(flock: Flock, stop: CancellationToken, home: String) -> Router {
    Router::new()
        .route(
            "/api/v1/flock",
            get(|State(feed): State<Feed>| async move { Json(feed.snapshot()) }),
        )
        .route("/api/v1/flock/live", get(upgrade))
        .with_state(Feed { flock, stop, home })
}

async fn upgrade(State(feed): State<Feed>, ws: WebSocketUpgrade) -> Response {
    ws.max_frame_size(1024)
        .max_message_size(1024)
        .on_upgrade(move |socket| live(socket, feed))
}

async fn live(mut socket: WebSocket, feed: Feed) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = feed.stop.cancelled() => return,
            incoming = socket.recv() => if !matches!(incoming, Some(Ok(Message::Ping(_) | Message::Pong(_)))) { return; },
            _ = tick.tick() => {
                let text = serde_json::to_string(&feed.snapshot()).expect("validated flock metrics");
                if !matches!(tokio::time::timeout(Duration::from_secs(2), socket.send(Message::Text(text.into()))).await, Ok(Ok(()))) { return; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pulse(incarnation: u64, sequence: u64, requests: u64) -> Heartbeat {
        Heartbeat {
            bird_id: "wing-1".into(),
            incarnation,
            sequence,
            pid: 7,
            state: "running".into(),
            requests,
            p95_ms: Some(12.0),
            ..Default::default()
        }
    }

    #[test]
    fn reconnects_do_not_duplicate_birds_or_freshen_replayed_counts() {
        let flock = Flock::default();
        let at = Instant::now();
        flock.record(pulse(1, 2, 8), at).unwrap();
        flock.record(pulse(1, 1, 3), at + STALE).unwrap();
        let birds = flock.view(at + STALE);
        assert_eq!(birds.len(), 1);
        assert!(birds[0].stale);
        assert_eq!(birds[0].requests, 8);
        flock.record(pulse(1, 3, 10), at + STALE).unwrap();
        assert!(!flock.view(at + STALE)[0].stale);
        assert_eq!(
            flock.record(pulse(2, 1, 0), at + STALE).unwrap_err().code(),
            tonic::Code::AlreadyExists
        );
        flock.record(pulse(2, 1, 0), at + 2 * STALE).unwrap();
        assert_eq!(flock.view(at + 2 * STALE)[0].requests, 0);
        assert!(flock.view(at + 2 * STALE + RETAIN).is_empty());
    }
}
