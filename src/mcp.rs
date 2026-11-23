use std::{collections::BTreeSet, sync::Arc, time::Duration};

use rmcp::{
    ClientLifecycleMode, ClientServiceExt, RoleClient, ServiceError,
    model::{
        CallToolRequestParams, ClientConfig, ClientRequest, ErrorCode, GetPromptRequestParams,
        Implementation, PaginatedRequestParams, ProtocolVersion, ServerResult,
    },
    service::{ClientInitializeError, RunningService},
    transport::{
        StreamableHttpClientTransport, common::client_side_sse::NeverRetry,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use crate::{
    config::{Method, Protocol, Target, Task},
    metrics::Outcome,
};

pub struct Session(RunningService<RoleClient, ClientConfig>);

impl Session {
    pub fn peer(&self) -> &rmcp::service::Peer<RoleClient> {
        self.0.peer()
    }
    pub async fn open(target: &Target, tasks: &[Task], timeout: Duration) -> Result<Self, Outcome> {
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(timeout);
        let endpoint = target.endpoint();
        if endpoint.scheme() == "http" {
            // A plaintext-only session needs no platform CA store and trusts no TLS peers.
            builder = builder.tls_certs_only([]);
        }
        let client = builder.build().map_err(|_| Outcome::Transport)?;
        let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint.as_str())
            .max_json_response_size(1024 * 1024)
            .max_sse_event_size(1024 * 1024)
            .max_error_response_size(0)
            .max_concurrent_requests(1)
            .control_request_timeout(timeout)
            .reinit_on_expired_session(false);
        config.retry_config = Arc::new(NeverRetry::default());
        let transport = StreamableHttpClientTransport::with_client(client, config);
        let (version, lifecycle) = match target.protocol {
            Protocol::Exact(v) if v.revision.has_initialize() => {
                (v.revision.clone(), ClientLifecycleMode::Initialize)
            }
            Protocol::Exact(v) => (
                v.revision.clone(),
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![v.revision.clone()],
                },
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
        if !target.protocol.accepts(&info.protocol_version) {
            return Err(Outcome::ProtocolVersion);
        }
        let capable = tasks.iter().all(|task| match task.method {
            Method::CallTool | Method::ListTools => info.capabilities.tools.is_some(),
            Method::GetPrompt | Method::ListPrompts => info.capabilities.prompts.is_some(),
        });
        if !capable {
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
        let mut missing: BTreeSet<_> = tasks.iter().filter_map(|t| t.tool.as_deref()).collect();
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
        let pointer = task.assert_exists.as_deref();
        match task.method {
            Method::CallTool => {
                let params = CallToolRequestParams::new(task.tool.clone().expect("validated tool"))
                    .with_arguments(task.arguments.clone());
                let result = self.0.call_tool(params).await.map_err(service_error)?;
                if result.is_error == Some(true) {
                    return Err(Outcome::ToolError);
                }
                check_result(result, pointer)
            }
            Method::GetPrompt => {
                let params =
                    GetPromptRequestParams::new(task.prompt.clone().expect("validated prompt"))
                        .with_arguments(task.arguments.clone());
                let result = self.0.get_prompt(params).await.map_err(service_error)?;
                check_result(result, pointer)
            }
            Method::ListTools | Method::ListPrompts => {
                // A scheduled list measures the server, not the SDK's discovery cache.
                let request = if task.method == Method::ListTools {
                    ClientRequest::ListToolsRequest(Default::default())
                } else {
                    ClientRequest::ListPromptsRequest(Default::default())
                };
                match (
                    task.method,
                    self.0.send_request(request).await.map_err(service_error)?,
                ) {
                    (Method::ListTools, ServerResult::ListToolsResult(result)) => {
                        check_result(result, pointer)
                    }
                    (Method::ListPrompts, ServerResult::ListPromptsResult(result)) => {
                        check_result(result, pointer)
                    }
                    _ => Err(Outcome::Protocol),
                }
            }
        }
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

fn check_result<T: serde::Serialize>(result: T, pointer: Option<&str>) -> Result<(), Outcome> {
    if let Some(pointer) = pointer {
        let json = serde_json::to_value(result).map_err(|_| Outcome::Protocol)?;
        if json.pointer(pointer).is_none() {
            return Err(Outcome::Assertion);
        }
    }
    Ok(())
}

fn init_error(error: ClientInitializeError) -> Outcome {
    match error {
        ClientInitializeError::TransportError { .. }
        | ClientInitializeError::ConnectionClosed(_) => Outcome::Transport,
        ClientInitializeError::JsonRpcError(err)
            if matches!(
                err.code,
                ErrorCode::UNSUPPORTED_PROTOCOL_VERSION | ErrorCode::METHOD_NOT_FOUND
            ) =>
        {
            Outcome::ProtocolVersion
        }
        ClientInitializeError::JsonRpcError(_) => Outcome::RpcError,
        ClientInitializeError::NoCompatibleProtocolVersion { .. } => Outcome::ProtocolVersion,
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
