//! The MCP service (rmcp) and its loopback HTTP server.
//!
//! Like Paper's server it listens on 127.0.0.1 only, accepts loopback `Host` headers only
//! (DNS rebinding guard), rejects browser requests that carry an `Origin`, repairs a missing
//! `Accept` header (some clients omit it and then misread the 406 as an auth failure) and
//! answers unknown routes with a plain JSON hint rather than an OAuth-looking error.

use super::{Call, ENDPOINT, Out, ToolResult, tools};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Hands a call to the editor thread. Returns false when the app is gone.
pub type Dispatch = Arc<dyn Fn(Call) -> bool + Send + Sync>;

/// Longest a tool call may take on the editor thread.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// The tool catalogue as rmcp tools.
pub fn tool_list() -> Vec<Tool> {
    tools::definitions()
        .into_iter()
        .map(|t| serde_json::from_value(t).expect("tool definitions are valid MCP tools"))
        .collect()
}

/// Server identity, capabilities and instructions.
pub fn info() -> ServerConfig {
    ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new("sketch-rs", env!("CARGO_PKG_VERSION")))
        .with_instructions(tools::INSTRUCTIONS)
}

/// Tool output as an MCP result (errors become `isError` results the agent can read).
pub fn to_result(r: ToolResult) -> CallToolResult {
    match r {
        Ok(outs) => CallToolResult::success(
            outs.into_iter()
                .map(|o| match o {
                    Out::Text(t) => ContentBlock::text(t),
                    Out::Image { data, mime } => {
                        ContentBlock::image(crate::assets::base64(&data), mime)
                    }
                })
                .collect(),
        ),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
    }
}

/// One MCP session's handler; all sessions share the dispatcher.
#[derive(Clone)]
pub struct SketchServer {
    dispatch: Dispatch,
}

impl SketchServer {
    pub fn new(dispatch: Dispatch) -> SketchServer {
        SketchServer { dispatch }
    }
}

impl ServerHandler for SketchServer {
    fn get_info(&self) -> ServerConfig {
        info()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let call = Call {
            name: request.name.to_string(),
            args: serde_json::Value::Object(request.arguments.unwrap_or_default()),
            reply,
        };
        let r = if !(self.dispatch)(call) {
            Err("Sketch is shutting down".to_string())
        } else {
            match tokio::time::timeout(CALL_TIMEOUT, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => Err("Sketch dropped the call (is it closing?)".into()),
                Err(_) => Err("Sketch did not answer in time (is a modal dialog open?)".into()),
            }
        };
        Ok(to_result(r).into())
    }
}

/// Add the `Accept` header the streamable HTTP transport requires when a client omits it.
async fn fix_accept(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header::{ACCEPT, HeaderValue};
    if req.method() == axum::http::Method::POST {
        let accept = req
            .headers()
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !(accept.contains("application/json") && accept.contains("text/event-stream")) {
            req.headers_mut().insert(
                ACCEPT,
                HeaderValue::from_static("application/json, text/event-stream"),
            );
        }
    }
    next.run(req).await
}

/// The HTTP routes: the MCP endpoint plus a JSON 404 for everything else.
pub fn router(dispatch: Dispatch, ct: CancellationToken) -> axum::Router {
    let config = StreamableHttpServerConfig::default()
        .enforce_origin_validation()
        .with_max_request_body_bytes(64 << 20)
        .with_cancellation_token(ct);
    let service: StreamableHttpService<SketchServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(SketchServer::new(dispatch.clone())),
            Default::default(),
            config,
        );
    axum::Router::new()
        .nest_service(ENDPOINT, service)
        .fallback(|| async {
            (
                axum::http::StatusCode::NOT_FOUND,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                format!(
                    "{{\"status\":\"not_found\",\"message\":\"Route not found. The MCP endpoint is {ENDPOINT}.\"}}"
                ),
            )
        })
        .layer(axum::middleware::from_fn(fix_accept))
}

/// A running HTTP server; stops when dropped.
pub struct Running {
    pub port: u16,
    ct: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    pub fn url(&self) -> String {
        super::url(self.port)
    }
    pub fn stop(mut self) {
        self.shutdown();
    }
    fn shutdown(&mut self) {
        self.ct.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Serve MCP on `127.0.0.1:port` (0 = any free port) from a background thread.
pub fn spawn(port: u16, dispatch: Dispatch) -> Result<Running, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            format!(
                "port {port} is in use (another Sketch instance?); set SKETCH_RS_MCP_PORT to use another"
            )
        } else {
            format!("cannot listen on 127.0.0.1:{port}: {e}")
        }
    })?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let ct = CancellationToken::new();
    let thread_ct = ct.clone();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("sketch-rs-mcp")
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let thread = std::thread::Builder::new()
        .name("sketch-rs-mcp-http".into())
        .spawn(move || {
            rt.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => l,
                    Err(e) => {
                        log::error!("mcp: {e}");
                        return;
                    }
                };
                let app = router(dispatch, thread_ct.child_token());
                if let Err(e) = axum::serve(listener, app)
                    .with_graceful_shutdown(thread_ct.cancelled_owned())
                    .await
                {
                    log::error!("mcp: server stopped: {e}");
                }
            });
        })
        .map_err(|e| e.to_string())?;
    Ok(Running {
        port,
        ct,
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_deserialize_and_names_are_unique() {
        let tools = tool_list();
        assert!(tools.len() >= 30, "{}", tools.len());
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "duplicate tool names");
        for t in &tools {
            assert_eq!(
                t.input_schema.get("type").and_then(|v| v.as_str()),
                Some("object")
            );
        }
        // Paper's core write and read tools are present under the same names.
        for name in [
            "get_basic_info",
            "get_selection",
            "get_screenshot",
            "write_html",
            "create_artboard",
            "update_styles",
            "set_text_content",
            "delete_nodes",
            "duplicate_nodes",
            "move_nodes",
            "get_jsx",
            "finish_working_on_nodes",
        ] {
            assert!(names.contains(&name), "missing {name}");
        }
    }
}
