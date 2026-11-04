//! `sketch-rs-mcp`: MCP over stdio for clients that can't speak HTTP, relayed to the running
//! Sketch app (`http://127.0.0.1:29980/mcp`, or `$SKETCH_RS_MCP_PORT`).
//!
//! The tool list is answered locally, so the agent sees every tool even before the app is
//! started; calls are forwarded to the app and report a clear error while it isn't running.
//! The connection is opened lazily and re-opened after the app restarts.

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ResultType, ServerConfig,
};
use rmcp::service::{RequestContext, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ErrorData, RoleClient, RoleServer, ServerHandler, ServiceExt};
use sketch_rs::mcp::{self, server};
use std::sync::Arc;
use tokio::sync::Mutex;

const USAGE: &str = "\
sketch-rs-mcp: stdio MCP bridge to the running Sketch app

USAGE:
  sketch-rs-mcp           serve MCP on stdin/stdout (what agent harnesses run)
  sketch-rs-mcp --url     print the HTTP endpoint it relays to
  sketch-rs-mcp --help

The app serves MCP itself at http://127.0.0.1:29980/mcp; set SKETCH_RS_MCP_PORT to change
the port (in both the app's and the bridge's environment).
";

type Client = RunningService<RoleClient, ()>;

#[derive(Clone)]
struct Bridge {
    url: String,
    client: Arc<Mutex<Option<Client>>>,
}

impl Bridge {
    async fn connect(&self) -> Result<(), String> {
        let transport = StreamableHttpClientTransport::from_uri(self.url.clone());
        let client = ()
            .serve(transport)
            .await
            .map_err(|e| format!("cannot reach Sketch at {}: {e}", self.url))?;
        *self.client.lock().await = Some(client);
        Ok(())
    }

    async fn forward(&self, params: CallToolRequestParams) -> Result<CallToolResult, String> {
        // One retry with a fresh connection covers an app restart between calls.
        for attempt in 0..2 {
            if self.client.lock().await.is_none() {
                self.connect().await?;
            }
            let res = {
                let guard = self.client.lock().await;
                guard.as_ref().unwrap().call_tool(params.clone()).await
            };
            match res {
                Ok(r) => return Ok(r),
                Err(e) if attempt == 0 => {
                    log::info!("sketch-rs-mcp: reconnecting after {e}");
                    *self.client.lock().await = None;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        unreachable!()
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        server::info()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(server::tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let r = match self.forward(request).await {
            Ok(mut r) => {
                // The app may have answered over an older protocol revision (no resultType);
                // 2026-07-28 clients require it. rmcp drops it again for older-revision peers.
                if r.result_type.is_none() {
                    r.result_type = Some(ResultType::COMPLETE);
                }
                r
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "{e}\nThe Sketch app is not running or its MCP server is off. Ask the user to \
                 start Sketch (it serves MCP while open), then try again."
            ))]),
        };
        Ok(r.into())
    }
}

fn main() -> std::process::ExitCode {
    // Logs go to stderr: stdout carries the protocol.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = mcp::url(mcp::port());
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return std::process::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--url") {
        println!("{url}");
        return std::process::ExitCode::SUCCESS;
    }
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("sketch-rs-mcp: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let r = rt.block_on(async move {
        let bridge = Bridge {
            url,
            client: Arc::new(Mutex::new(None)),
        };
        let service = bridge
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| e.to_string())?;
        service.waiting().await.map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    });
    match r {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sketch-rs-mcp: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
