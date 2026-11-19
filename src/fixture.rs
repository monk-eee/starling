use std::time::Duration;

use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, JsonObject,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

struct Fixture;

impl ServerHandler for Fixture {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let empty = JsonObject::from_iter([("type".to_owned(), json!("object"))]);
        let mut echo = empty.clone();
        echo.insert("properties".into(), json!({"text": {"type": "string"}}));
        echo.insert("required".into(), json!(["text"]));
        Ok(ListToolsResult::with_all_items(vec![
            Tool::new("echo", "Return the input text.", echo),
            Tool::new("status", "Return a readiness message.", empty.clone()),
            Tool::new("reject", "Return a semantic tool error.", empty),
        ]))
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        tokio::time::sleep(Duration::from_millis(12)).await;
        let text = match req.name.as_ref() {
            "echo" => req
                .arguments
                .as_ref()
                .and_then(|a| a.get("text"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| ErrorData::invalid_params("expected text", None))?,
            "status" => "ready",
            "reject" => {
                return Ok(CallToolResult::error(vec![ContentBlock::text("rejected")]).into());
            }
            _ => return Err(ErrorData::invalid_params("unknown tool", None)),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text.to_owned())]).into())
    }
}

pub async fn serve(listener: TcpListener, shutdown: CancellationToken) -> anyhow::Result<()> {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts([listener.local_addr()?.ip().to_string(), "localhost".into()])
        .enforce_origin_validation()
        .with_json_response(true)
        .with_max_request_body_bytes(128 * 1024)
        .with_cancellation_token(shutdown.child_token());
    let service = StreamableHttpService::<_, LocalSessionManager>::new(
        || Ok(Fixture),
        Default::default(),
        config,
    );
    axum::serve(listener, axum::Router::new().nest_service("/mcp", service))
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await?;
    Ok(())
}
