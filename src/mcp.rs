use std::{collections::BTreeSet, sync::Arc, time::Duration};

use rmcp::{
    ClientLifecycleMode, ClientServiceExt, RoleClient, ServiceError,
    model::{
        CallToolRequestParams, ClientConfig, Implementation, PaginatedRequestParams,
        ProtocolVersion,
    },
    service::{ClientInitializeError, RunningService},
    transport::{
        StreamableHttpClientTransport, common::client_side_sse::NeverRetry,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use crate::{
    config::{Protocol, Target, Task},
    metrics::Outcome,
};

pub struct Session(RunningService<RoleClient, ClientConfig>);

impl Session {
    pub async fn open(target: &Target, timeout: Duration) -> Result<Self, Outcome> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(timeout)
            .build()
            .map_err(|_| Outcome::Transport)?;
        let mut config = StreamableHttpClientTransportConfig::with_uri(target.url.as_str())
            .max_json_response_size(1024 * 1024)
            .max_sse_event_size(1024 * 1024)
            .max_error_response_size(0)
            .max_concurrent_requests(1)
            .control_request_timeout(timeout)
            .reinit_on_expired_session(false);
        config.retry_config = Arc::new(NeverRetry::default());
        let transport = StreamableHttpClientTransport::with_client(client, config);
        let (version, lifecycle) = match target.protocol {
            Protocol::Initialize => (
                ProtocolVersion::V_2025_11_25,
                ClientLifecycleMode::Initialize,
            ),
            Protocol::Auto => (
                ProtocolVersion::V_2026_07_28,
                ClientLifecycleMode::Auto {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                    legacy_version: Some(ProtocolVersion::V_2025_11_25),
                },
            ),
        };
        let client_info = ClientConfig::new(
            Default::default(),
            Implementation::new("starling", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(version);
        let peer = client_info
            .serve_with_lifecycle(transport, lifecycle)
            .await
            .map_err(init_error)?;
        let info = peer.peer_info().ok_or(Outcome::Protocol)?;
        let supported = info.protocol_version == ProtocolVersion::V_2025_11_25
            || (matches!(target.protocol, Protocol::Auto)
                && info.protocol_version == ProtocolVersion::V_2026_07_28);
        if info.capabilities.tools.is_none() || !supported {
            return Err(Outcome::Capability);
        }
        Ok(Self(peer))
    }

    pub fn version(&self) -> String {
        self.0
            .peer_info()
            .expect("checked during initialization")
            .protocol_version
            .to_string()
    }

    pub async fn discover(&self, tasks: &[Task]) -> Result<(), Outcome> {
        let mut missing: BTreeSet<_> = tasks.iter().map(|t| t.tool.as_str()).collect();
        let mut cursor = None;
        for _ in 0..16 {
            let page = self
                .0
                .list_tools(
                    cursor
                        .take()
                        .map(|c| PaginatedRequestParams::default().with_cursor(Some(c))),
                )
                .await
                .map_err(service_error)?;
            for tool in &page.tools {
                missing.remove(tool.name.as_ref());
            }
            if missing.is_empty() {
                return Ok(());
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Err(Outcome::Capability);
            }
        }
        Err(Outcome::Protocol)
    }

    pub async fn call(&self, task: &Task) -> Result<(), Outcome> {
        let params =
            CallToolRequestParams::new(task.tool.clone()).with_arguments(task.arguments.clone());
        let result = self.0.call_tool(params).await.map_err(service_error)?;
        if result.is_error == Some(true) {
            return Err(Outcome::ToolError);
        }
        if let Some(pointer) = &task.assert_exists {
            let json = serde_json::to_value(result).map_err(|_| Outcome::Protocol)?;
            if json.pointer(pointer).is_none() {
                return Err(Outcome::Assertion);
            }
        }
        Ok(())
    }

    pub async fn close(mut self, timeout: Duration) -> Result<(), Outcome> {
        self.0
            .close_with_timeout(timeout)
            .await
            .map_err(|_| Outcome::Transport)?
            .map(|_| ())
            .ok_or(Outcome::Timeout)
    }
}

fn init_error(error: ClientInitializeError) -> Outcome {
    match error {
        ClientInitializeError::TransportError { .. }
        | ClientInitializeError::ConnectionClosed(_) => Outcome::Transport,
        ClientInitializeError::JsonRpcError(_) => Outcome::RpcError,
        ClientInitializeError::NoCompatibleProtocolVersion { .. } => Outcome::Capability,
        ClientInitializeError::LegacyFallbackFailed { fallback, .. } => init_error(*fallback),
        ClientInitializeError::Cancelled => Outcome::Cancelled,
        _ => Outcome::Protocol,
    }
}

fn service_error(error: ServiceError) -> Outcome {
    match error {
        ServiceError::McpError(_) => Outcome::RpcError,
        ServiceError::TransportSend(_) | ServiceError::TransportClosed => Outcome::Transport,
        ServiceError::Timeout { .. } => Outcome::Timeout,
        ServiceError::Cancelled { .. } => Outcome::Cancelled,
        _ => Outcome::Protocol,
    }
}
