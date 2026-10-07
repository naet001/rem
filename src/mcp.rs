//! MCP client config: `~/.config/rem/mcp.json` in Claude `mcpServers`
//! shape (ADR-0013). Missing file = zero servers. Invalid entries warn
//! and skip. `${VAR}` / `${VAR:-default}` expansion everywhere.

use std::collections::HashMap;
use std::path::Path;

use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};

/// Canonical path to the MCP servers file (`~/.config/rem/mcp.json`),
/// beside the main config file. Single source of truth.
pub fn default_path() -> Result<std::path::PathBuf, String> {
    let config_dir = dirs::config_dir().ok_or("could not determine platform config directory")?;
    Ok(config_dir.join("rem").join("mcp.json"))
}

/// Load from the default path. Missing file = zero servers, no error.
/// Warnings cover unreadable files, invalid JSON, and skipped entries.
/// Startup never fails because of MCP (warn-and-continue).
pub fn load() -> LoadedConfig {
    match default_path() {
        Ok(p) => load_from_path(&p),
        Err(e) => LoadedConfig {
            warnings: vec![format!("mcp: {e}")],
            ..Default::default()
        },
    }
}

/// Default token store for production paths (login/logout/startup).
/// Falls back to a local file when the config dir is unavailable.
#[allow(dead_code)]
pub(crate) fn default_token_store() -> FileTokenStore {
    FileTokenStore::new(
        tokens_path().unwrap_or_else(|_| std::path::PathBuf::from("mcp-tokens.json")),
    )
}

/// Canonical path to the OAuth token file
/// (`~/.config/rem/mcp-tokens.json`), beside `mcp.json`.
pub fn tokens_path() -> Result<std::path::PathBuf, String> {
    let config_dir = dirs::config_dir().ok_or("could not determine platform config directory")?;
    Ok(config_dir.join("rem").join("mcp-tokens.json"))
}

/// One server's stored OAuth token (access + refresh + scopes).
/// Never logged — `Debug` is redacted by hand.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct StoredMcpToken {
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

/// File-backed per-server OAuth token store. One JSON map
/// (`server -> token`) at 0600, written atomically (tmp + rename).
/// Tokens never reach logs or transcripts — only the `Authorization`
/// header wire value.
#[derive(Debug, Clone)]
pub(crate) struct FileTokenStore {
    path: std::path::PathBuf,
}

impl FileTokenStore {
    #[cfg_attr(not(test), allow(dead_code))]
    fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    fn read_all(&self) -> std::collections::HashMap<String, StoredMcpToken> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    fn write_all(
        &self,
        map: &std::collections::HashMap<String, StoredMcpToken>,
    ) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mcp tokens: mkdir failed: {e}"))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let content =
            serde_json::to_string(map).map_err(|e| format!("mcp tokens: encode failed: {e}"))?;
        std::fs::write(&tmp, content).map_err(|e| format!("mcp tokens: write failed: {e}"))?;
        chmod_0600(&tmp)?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("mcp tokens: rename failed: {e}"))?;
        chmod_0600(&self.path)?;
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn load_sync(&self, server: &str) -> Option<StoredMcpToken> {
        self.read_all().get(server).cloned()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn save_sync(&self, server: &str, token: &str, scopes: &[String]) -> Result<(), String> {
        let mut map = self.read_all();
        let refresh_token = map.get(server).and_then(|t| t.refresh_token.clone());
        map.insert(
            server.to_string(),
            StoredMcpToken {
                access_token: token.to_string(),
                refresh_token,
                scopes: scopes.to_vec(),
            },
        );
        self.write_all(&map)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn save_full_sync(&self, server: &str, token: StoredMcpToken) -> Result<(), String> {
        let mut map = self.read_all();
        map.insert(server.to_string(), token);
        self.write_all(&map)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn clear_sync(&self, server: &str) {
        let mut map = self.read_all();
        if map.remove(server).is_some() {
            let _ = self.write_all(&map);
        }
    }
}

/// Build an oauth2 access token value (rmcp's `OAuthTokenResponse`
/// constructor takes these).
fn oauth2_access_token(secret: &str) -> oauth2::AccessToken {
    oauth2::AccessToken::new(secret.to_string())
}

/// Bearer token type marker for token-response construction.
fn oauth2_bearer() -> oauth2::basic::BasicTokenType {
    oauth2::basic::BasicTokenType::Bearer
}

/// Adapter: my per-server file token store behind rmcp's
/// `CredentialStore` trait, scoped to one server name. The
/// `AuthorizationManager` reads/writes through this during
/// token refresh; the map file stays the single source of truth.
#[derive(Debug, Clone)]
struct StoreCredentialBridge {
    store: FileTokenStore,
    server: String,
}

#[async_trait::async_trait]
impl rmcp::transport::auth::CredentialStore for StoreCredentialBridge {
    async fn load(
        &self,
    ) -> Result<Option<rmcp::transport::auth::StoredCredentials>, rmcp::transport::auth::AuthError>
    {
        Ok(self.store.load_sync(&self.server).map(|t| {
            use rmcp::transport::auth::OAuthTokenResponse;
            // Rebuild a bearer token response from the stored access
            // token. No expiry recorded (slice 2 login stamps it);
            // the manager returns no-expiry tokens with zero network.
            let response = OAuthTokenResponse::new(
                oauth2_access_token(&t.access_token),
                oauth2_bearer(),
                Default::default(),
            );
            rmcp::transport::auth::StoredCredentials::new(
                format!("rem-mcp-{}", self.server),
                Some(response),
                t.scopes.clone(),
                None,
            )
        }))
    }

    async fn save(
        &self,
        credentials: rmcp::transport::auth::StoredCredentials,
    ) -> Result<(), rmcp::transport::auth::AuthError> {
        // Persist refreshed tokens back to the file store (access +
        // refresh when present). Slice 2 login writes the initial row.
        let access = credentials
            .token_response
            .as_ref()
            .map(|r| {
                use oauth2::TokenResponse;
                r.access_token().secret().to_string()
            })
            .unwrap_or_default();
        if access.is_empty() {
            return Ok(());
        }
        self.store
            .save_full_sync(
                &self.server,
                StoredMcpToken {
                    access_token: access,
                    refresh_token: None,
                    scopes: credentials.granted_scopes.clone(),
                },
            )
            .map_err(rmcp::transport::auth::AuthError::InternalError)
    }

    async fn clear(&self) -> Result<(), rmcp::transport::auth::AuthError> {
        self.store.clear_sync(&self.server);
        Ok(())
    }
}

/// Restrict a path to owner read/write (0600). No-op on non-unix.
fn chmod_0600(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("mcp tokens: chmod failed: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Compose a Claude-style tool name: `mcp__<server>__<tool>`. Bare
/// upstream names are never exposed.
#[allow(dead_code)]
pub fn tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{}__{tool}", sanitize_server_name(server))
}

/// One remote tool definition (name + description + JSON schema),
/// decoupled from rmcp types so tests use fakes and production maps
/// from `list_tools` results.
#[derive(Debug, Clone)]
struct RemoteToolDef {
    name: String,
    description: String,
    schema: serde_json::Value,
}

/// Production peer: the real rmcp client handle. `Clone` (cheap) +
/// `Send + Sync`, so it satisfies the bridge seam with no adapter state.
#[derive(Debug, Clone)]
struct RmcpPeer(rmcp::service::Peer<rmcp::service::RoleClient>);

impl RmcpPeer {
    /// True once the underlying transport can no longer accept calls.
    fn is_closed(&self) -> bool {
        self.0.is_transport_closed()
    }

    /// List all remote tool definitions over the wire.
    async fn list_tools(&self) -> Result<Vec<RemoteToolDef>, String> {
        self.0
            .list_all_tools()
            .await
            .map(|tools| {
                tools
                    .into_iter()
                    .map(|t| RemoteToolDef {
                        name: t.name.to_string(),
                        description: t
                            .description
                            .as_ref()
                            .map(|d| d.to_string())
                            .unwrap_or_default(),
                        schema: t.schema_as_json_value(),
                    })
                    .collect()
            })
            .map_err(|e| format!("MCP list_tools failed: {e}"))
    }
}

impl RmcpPeer {
    /// Call one remote tool by bare upstream name.
    async fn call_tool(
        &self,
        bare_name: String,
        args: serde_json::Value,
    ) -> Result<String, String> {
        use rmcp::model::CallToolRequestParams;
        let arguments = match args {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return Err(format!(
                    "MCP tool `{bare_name}` needs an object argument, got: {other}"
                ));
            }
        };
        let mut params = CallToolRequestParams::new(bare_name.clone());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        match self.0.call_tool(params).await {
            Ok(result) if result.is_error == Some(true) => Err(render_content(&result.content)),
            Ok(result) => Ok(render_content(&result.content)),
            Err(e) => Err(format!("MCP tool `{bare_name}` failed: {e}")),
        }
    }
}

/// Preamble index of connected MCP tools, mirroring the skills index
/// shape: one line per `mcp__<server>__<tool>` with its description.
/// Empty when no servers. The Principal reads this to know when to
/// reach for MCP tools. `connect_all` reports are already sorted.
pub fn preamble_section(inventory: &[ServerInventory]) -> String {
    if inventory.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\nAvailable MCP tools (server — mcp__server__tool — description). \
         These are third-party tools; every call asks for approval. \
         Prefer reading relevant files first, then call the MCP tool:\n",
    );
    for server in inventory {
        for tool in &server.tools {
            out.push_str(&format!(
                "- {} — {} — {}\n",
                server.server,
                tool_name(&server.server, &tool.bare_name),
                tool.description
            ));
        }
    }
    out
}

/// Render MCP content blocks as plain text for the model transcript.
/// Text concatenates; anything else falls back to its JSON shape.
fn render_content(blocks: &[rmcp::model::ContentBlock]) -> String {
    let mut out = String::new();
    for b in blocks {
        if let Some(t) = b.as_text() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&t.text);
        } else if let Ok(v) = serde_json::to_value(b) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&v.to_string());
        }
    }
    out
}

/// Fake MCP server for bridge tests: serves one tool over duplex,
/// mirroring rig-agent's own ScenarioServer prior art.
#[cfg(test)]
#[derive(Clone)]
struct FakeServer {
    instructions: Option<String>,
}

#[cfg(test)]
impl FakeServer {
    fn plain() -> Self {
        Self { instructions: None }
    }
    fn with_instructions(instructions: &str) -> Self {
        Self {
            instructions: Some(instructions.to_string()),
        }
    }
}

/// Live connection status for one MCP server.
#[derive(Debug, Clone)]
pub(crate) enum McpConnStatus {
    /// Spawned/connected and tools registered. Panel counts read live
    /// off the shared handle at display time.
    Connected,
    /// Spawn/connect/list failed; carries the warn-and-continue reason.
    Failed { reason: String },
    /// HTTP 401 path (Ticket 06): needs `/mcp login <name>`.
    #[allow(dead_code)]
    NeedsAuth,
}

/// One MCP server's live record: status + session enable flag +
/// transport (for reconnect on toggle-on) + held connection.
/// No `Debug`: `Connection` holds a task handle.
pub(crate) struct McpServerRecord {
    pub(crate) name: String,
    pub(crate) status: McpConnStatus,
    pub(crate) enabled: bool,
    pub(crate) transport: Transport,
    pub(crate) connection: Option<Connection>,
}

/// Shared live MCP state: one record per configured server.
/// Owned by `RigAgent`, cloned into each `RefreshHandler` so
/// `list_changed` refresh skips session-disabled servers.
pub(crate) type SharedMcpState = std::sync::Arc<std::sync::Mutex<Vec<McpServerRecord>>>;

/// Client-side refresh handler: re-lists and reconciles one server's
/// prefixed tools on `notifications/tools/list_changed`, without
/// touching other servers' registrations. Skips session-disabled
/// servers so a toggle-off stays off across notifications.
#[derive(Clone)]
struct RefreshHandler {
    server: String,
    handle: rig::agent::tool::server::ToolServerHandle,
    /// Session state for the toggle-off skip. `None` in unit tests
    /// (always refresh); production passes the shared records.
    mcp: Option<SharedMcpState>,
    /// Token store for the 401 refresh-once path. `None` disables
    /// refresh handling (unit tests without records).
    store: Option<FileTokenStore>,
}

impl RefreshHandler {
    #[cfg_attr(not(test), allow(dead_code))]
    fn new(
        server: String,
        handle: rig::agent::tool::server::ToolServerHandle,
        mcp: SharedMcpState,
    ) -> Self {
        Self::with_store(server, handle, mcp, default_token_store())
    }

    fn with_store(
        server: String,
        handle: rig::agent::tool::server::ToolServerHandle,
        mcp: SharedMcpState,
        store: FileTokenStore,
    ) -> Self {
        Self {
            server,
            handle,
            mcp: Some(mcp),
            store: Some(store),
        }
    }

    #[cfg(test)]
    fn test_handler(server: &str, handle: &rig::agent::tool::server::ToolServerHandle) -> Self {
        Self {
            server: server.to_string(),
            handle: handle.clone(),
            mcp: None,
            store: None,
        }
    }
}

impl RefreshHandler {
    /// Connect over any client transport, then register this server's
    /// prefixed tools. Spawn / connect / list failures are `Err` with a
    /// human-readable reason — the caller warns and continues.
    async fn serve<T, E, A>(
        self,
        transport: T,
    ) -> Result<rmcp::service::RunningService<rmcp::service::RoleClient, Self>, String>
    where
        T: rmcp::transport::IntoTransport<rmcp::service::RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let service = rmcp::service::serve_client(self.clone(), transport)
            .await
            .map_err(|e| format!("mcp `{}`: connect failed: {e}", self.server))?;
        // Session-disabled servers stay off: skip initial registration
        // too, so a reconnect while disabled can't re-register. The
        // toggle-on path flips the flag before connecting.
        let enabled = self
            .mcp
            .as_ref()
            .and_then(|mcp| {
                mcp.lock().ok().and_then(|records| {
                    records
                        .iter()
                        .find(|r| r.name == self.server)
                        .map(|r| r.enabled)
                })
            })
            .unwrap_or(true);
        if enabled {
            let peer = service.peer().clone();
            let instructions = peer.peer_info().and_then(|info| info.instructions.clone());
            let auth_ctx = match (&self.mcp, &self.store) {
                (Some(m), Some(s)) => Some((m.clone(), s.clone())),
                _ => None,
            };
            reconcile_server_tools_with_state(
                &self.server,
                &self.handle,
                &peer,
                instructions.as_deref(),
                auth_ctx,
            )
            .await?;
        }
        Ok(service)
    }
}

impl rmcp::ClientHandler for RefreshHandler {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::default()
    }

    async fn on_tool_list_changed(
        &self,
        context: rmcp::service::NotificationContext<rmcp::service::RoleClient>,
    ) {
        // Session-disabled servers stay off: skip refresh entirely.
        // `None` (unit tests) always refreshes.
        let enabled = self
            .mcp
            .as_ref()
            .and_then(|mcp| {
                mcp.lock().ok().and_then(|records| {
                    records
                        .iter()
                        .find(|r| r.name == self.server)
                        .map(|r| r.enabled)
                })
            })
            .unwrap_or(true);
        if !enabled {
            return;
        }
        let instructions = context
            .peer
            .peer_info()
            .and_then(|info| info.instructions.clone());
        let auth_ctx = match (&self.mcp, &self.store) {
            (Some(m), Some(s)) => Some((m.clone(), s.clone())),
            _ => None,
        };
        let _ = reconcile_server_tools_with_state(
            &self.server,
            &self.handle,
            &context.peer,
            instructions.as_deref(),
            auth_ctx,
        )
        .await;
    }
}

/// Re-list one server's tools and reconcile its `mcp__<server>__*`
/// registrations: add new, refresh changed, remove vanished — other
/// servers' tools untouched. Server instructions prepend to each tool
/// description so the model knows when to reach for them.
#[cfg(test)]
async fn reconcile_server_tools(
    server: &str,
    handle: &rig::agent::tool::server::ToolServerHandle,
    peer: &rmcp::service::Peer<rmcp::service::RoleClient>,
    instructions: Option<&str>,
) -> Result<(), String> {
    reconcile_server_tools_with_state(server, handle, peer, instructions, None).await
}

/// Reconcile with optional shared state + token store for the 401
/// path: auth-shaped dispatch failures refresh-once, retry-once, then
/// mark needs-auth. `None` state disables the 401 handling (unit
/// tests without records).
#[allow(clippy::too_many_arguments)]
async fn reconcile_server_tools_with_state(
    server: &str,
    handle: &rig::agent::tool::server::ToolServerHandle,
    peer: &rmcp::service::Peer<rmcp::service::RoleClient>,
    instructions: Option<&str>,
    auth_ctx: Option<(SharedMcpState, FileTokenStore)>,
) -> Result<(), String> {
    let rpeer = RmcpPeer(peer.clone());
    let defs = rpeer.list_tools().await?;
    // Remove this server's stale registrations first.
    let prefix = format!("mcp__{}__", sanitize_server_name(server));
    let current: Vec<String> = handle
        .get_tool_defs(None)
        .await
        .map_err(|e| format!("mcp `{server}`: tool defs failed: {e}"))
        .map(|defs| defs.into_iter().map(|d| d.name).collect())?;
    for name in current.iter().filter(|n| n.starts_with(&prefix)) {
        handle.remove_tool(name).await;
    }
    let peer_arc = std::sync::Arc::new(rpeer);
    let server_owned = server.to_string();
    // Owned 401 context for the dispatch closures: state for marking
    // needs-auth + store for the refresh attempt. `None` disables.
    let auth_ctx_owned: Option<(SharedMcpState, FileTokenStore)> = auth_ctx;
    for def in &defs {
        let name = tool_name(server, &def.name);
        let server_for_tool = server_owned.clone();
        let auth_for_tool = auth_ctx_owned.clone();
        let description = match instructions {
            Some(i) => format!("{i}\n\n{}", def.description),
            None => def.description.clone(),
        };
        let schema = def.schema.clone();
        let bare = def.name.clone();
        let peer_for_tool = peer_arc.clone();
        let handle_for_tool = handle.clone();
        let name_for_tool = name.clone();
        let tool = DynamicTool::new(name, description, schema, move |_ctx, args| {
            let peer = peer_for_tool.clone();
            let bare = bare.clone();
            let handle_for_tool = handle_for_tool.clone();
            let name_for_tool = name_for_tool.clone();
            let server_for_tool = server_for_tool.clone();
            let auth_for_tool = auth_for_tool.clone();
            Box::pin(async move {
                // Self-retire on a dead transport: Rig only auto-retires
                // `is_live() == false` registrations, which
                // runtime-defined tools never report — so check here and
                // remove ourselves, returning a replannable error the
                // model can plan around instead of hanging.
                if peer.is_closed() {
                    handle_for_tool.remove_tool(&name_for_tool).await;
                    return Err(ToolExecutionError::not_found(format!(
                        "MCP server `{server_for_tool}` disconnected: `{name_for_tool}` retired, replan without it"
                    )));
                }
                match peer.call_tool(bare.clone(), args.clone()).await {
                    Ok(out) => Ok(ToolOutput::text(out)),
                    Err(first_err) => {
                        // 401 path: refresh once, retry once, then mark
                        // needs-auth with a server-naming model error.
                        // Non-auth failures pass through untouched.
                        if !is_auth_failure(&first_err) {
                            return Err(ToolExecutionError::other(first_err));
                        }
                        let Some((ref state, ref store)) = auth_for_tool else {
                            return Err(ToolExecutionError::other(first_err));
                        };
                        match try_refresh_token_once(state, store, &server_for_tool).await {
                            Ok(()) => match peer.call_tool(bare, args).await {
                                Ok(out) => Ok(ToolOutput::text(out)),
                                Err(retry_err) => {
                                    retire_server_tools(&handle_for_tool, &server_for_tool).await;
                                    let msg = auth_failure_needs_auth(
                                        state,
                                        &server_for_tool,
                                        &retry_err,
                                    )
                                    .expect_err("always errors");
                                    Err(ToolExecutionError::other(msg))
                                }
                            },
                            Err(_) => {
                                retire_server_tools(&handle_for_tool, &server_for_tool).await;
                                let msg =
                                    auth_failure_needs_auth(state, &server_for_tool, &first_err)
                                        .expect_err("always errors");
                                Err(ToolExecutionError::other(msg))
                            }
                        }
                    }
                }
            })
        });
        handle.add_dynamic_tool(tool).await;
    }
    Ok(())
}

#[cfg(test)]
impl rmcp::ServerHandler for FakeServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let info = rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_protocol_version(rmcp::model::ProtocolVersion::LATEST)
        .with_server_info(rmcp::model::Implementation::new("rem-mcp-test", "0.1.0"));
        match &self.instructions {
            Some(i) => info.with_instructions(i.clone()),
            None => info,
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::model::ErrorData> {
        #[allow(unused_imports)]
        use rmcp::ServiceExt as _;
        use std::sync::Arc;
        Ok(rmcp::model::ListToolsResult {
            tools: vec![rmcp::model::Tool::new(
                "create_issue",
                "Create an issue",
                Arc::new(serde_json::Map::new()),
            )],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
        let title = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::text(format!("wire:{}:{title}", request.name)),
        ]))
    }
}

#[cfg(test)]
async fn fake_running_client() -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, rmcp::model::ClientInfo>,
    tokio::task::JoinHandle<()>,
) {
    use rmcp::ServiceExt;
    let (client_to_server, server_from_client) = tokio::io::duplex(8192);
    let (server_to_client, client_from_server) = tokio::io::duplex(8192);
    let task = tokio::spawn(async move {
        let running = FakeServer::plain()
            .serve((server_from_client, server_to_client))
            .await
            .expect("server start");
        let _ = running.waiting().await;
    });
    let client = rmcp::model::ClientInfo::default()
        .serve((client_from_server, client_to_server))
        .await
        .expect("client connect");
    (client, task)
}

/// Hold one connected MCP server for the process lifetime. Dropping
/// this closes the transport and retires the server's tools.
pub(crate) struct Connection {
    _service: rmcp::service::RunningService<rmcp::service::RoleClient, RefreshHandler>,
}

/// Adopt one connected server into the shared records: status +
/// enabled flag + transport (for reconnect on toggle-on) + held
/// connection. Connected tool counts come from the inventory read.
fn adopt_connected(state: &SharedMcpState, cfg: &ServerConfig, conn: Connection) {
    let mut records = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    records.retain(|r| r.name != cfg.name);
    records.push(McpServerRecord {
        name: cfg.name.clone(),
        status: McpConnStatus::Connected,
        enabled: true,
        transport: cfg.transport.clone(),
        connection: Some(conn),
    });
}

/// Remove one server's `mcp__<server>__*` tools from the shared
/// handle. Shared by toggle-off, logout, drop-retire, and 401 paths.
async fn retire_server_tools(handle: &rig::agent::tool::server::ToolServerHandle, server: &str) {
    let prefix = format!("mcp__{}__", sanitize_server_name(server));
    let current: Vec<String> = handle
        .get_tool_defs(None)
        .await
        .map(|defs| defs.into_iter().map(|d| d.name).collect())
        .unwrap_or_default();
    for tool in current.iter().filter(|n| n.starts_with(&prefix)) {
        handle.remove_tool(tool).await;
    }
}

/// Attempt one token refresh for a 401'd server: rebuild a manager
/// from the record's URL + bridge store, call `refresh_token()`
/// (persists via the bridge on success). `Ok` means retry the call;
/// `Err` means fall through to needs-auth. Never throws.
async fn try_refresh_token_once(
    state: &SharedMcpState,
    store: &FileTokenStore,
    server: &str,
) -> Result<(), String> {
    let url = {
        let records = state
            .lock()
            .map_err(|_| "mcp state lock poisoned".to_string())?;
        match records.iter().find(|r| r.name == server) {
            Some(r) => match &r.transport {
                Transport::StreamableHttp { url, .. } => url.clone(),
                Transport::Stdio { .. } => return Err("stdio has no refresh".to_string()),
            },
            None => return Err(format!("no MCP server `{server}`")),
        }
    };
    let mut manager = rmcp::transport::auth::AuthorizationManager::new(url.as_str())
        .await
        .map_err(|e| format!("refresh setup failed: {e}"))?;
    manager.set_credential_store(StoreCredentialBridge {
        store: store.clone(),
        server: server.to_string(),
    });
    // Skip metadata discovery: refresh posts to the known token
    // endpoint from the stored client registration... in practice the
    // manager needs its oauth_client configured, which only discovery
    // + registration provide. Without them this errors and the caller
    // marks needs-auth — the safe direction.
    manager
        .refresh_token()
        .await
        .map(|_| ())
        .map_err(|e| format!("refresh failed: {e}"))
}

/// True when a dispatch failure looks auth-shaped: HTTP 401 markers,
/// rmcp auth errors, or tool-reported unauthorized text.
fn is_auth_failure(message: &str) -> bool {
    let low = message.to_lowercase();
    low.contains("401")
        || low.contains("unauthorized")
        || low.contains("invalid_token")
        || low.contains("insufficient_scope")
        || low.contains("authorizationrequired")
        || low.contains("authorization required")
}

/// Handle an auth-shaped dispatch failure for one server: mark
/// needs-auth, retire its tools, and return a model-visible error
/// naming the server with a login hint (replan, don't hang).
/// Returns `Err` always — the message IS the model feedback.
fn auth_failure_needs_auth(
    state: &SharedMcpState,
    server: &str,
    failure: &str,
) -> Result<String, String> {
    if let Ok(mut records) = state.lock()
        && let Some(r) = records.iter_mut().find(|r| r.name == server)
    {
        r.status = McpConnStatus::NeedsAuth;
        r.connection = None;
    }
    Err(format!(
        "MCP server `{server}` needs re-authentication ({failure}): run /mcp login {server} to reconnect, then retry or replan without its tools"
    ))
}

/// Adopt one OAuth-pending server into the shared records: the `/mcp`
/// panel shows needs-auth with a login hint. Toggle-on retries.
fn adopt_needs_auth(state: &SharedMcpState, cfg: &ServerConfig) {
    let mut records = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    records.retain(|r| r.name != cfg.name);
    records.push(McpServerRecord {
        name: cfg.name.clone(),
        status: McpConnStatus::NeedsAuth,
        enabled: true,
        transport: cfg.transport.clone(),
        connection: None,
    });
}

/// Adopt one failed server into the shared records: reason for the
/// `/mcp` panel, still toggleable (toggle-on retries the connect).
fn adopt_failed(state: &SharedMcpState, cfg: &ServerConfig, reason: String) {
    let mut records = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    records.retain(|r| r.name != cfg.name);
    records.push(McpServerRecord {
        name: cfg.name.clone(),
        status: McpConnStatus::Failed { reason },
        enabled: true,
        transport: cfg.transport.clone(),
        connection: None,
    });
}

/// Session-scope toggle for one MCP server. Off: drop the connection
/// and remove its `mcp__<server>__*` tools from the shared handle
/// (refresh skips disabled servers, so they stay off). On: reconnect
/// via the recorded transport and re-register. Returns a human
/// summary for the `/mcp` notice.
pub(crate) async fn mcp_set_enabled(
    state: &SharedMcpState,
    handle: &rig::agent::tool::server::ToolServerHandle,
    name: &str,
    enabled: bool,
) -> Result<String, String> {
    // Snapshot the transport without holding the lock across I/O.
    let transport = {
        let records = state
            .lock()
            .map_err(|_| "mcp state lock poisoned".to_string())?;
        match records.iter().find(|r| r.name == name) {
            Some(r) => r.transport.clone(),
            None => return Err(format!("no MCP server `{name}`")),
        }
    };
    if !enabled {
        // Drop the connection first, then remove tools.
        if let Ok(mut records) = state.lock()
            && let Some(r) = records.iter_mut().find(|r| r.name == name)
        {
            r.connection = None;
            r.enabled = false;
            // Status stays Connected; the panel reads count 0 live
            // once tools are removed.
        }
        let prefix = format!("mcp__{}__", sanitize_server_name(name));
        let current: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .map_err(|e| format!("mcp `{name}`: tool defs failed: {e}"))
            .map(|defs| defs.into_iter().map(|d| d.name).collect())?;
        for tool in current.iter().filter(|n| n.starts_with(&prefix)) {
            handle.remove_tool(tool).await;
        }
        return Ok(format!("mcp `{name}` off this session"));
    }
    // Toggle-on: flip the flag FIRST so the fresh handler's serve()
    // registers (serve skips while disabled). Reconnect through the
    // recorded transport.
    if let Ok(mut records) = state.lock()
        && let Some(r) = records.iter_mut().find(|r| r.name == name)
    {
        r.enabled = true;
    }
    let cfg = ServerConfig {
        name: name.to_string(),
        transport,
    };
    let conn = match &cfg.transport {
        Transport::Stdio { .. } => connect_stdio(&cfg, handle, state).await,
        Transport::StreamableHttp { .. } => connect_http(&cfg, handle, state).await,
    };
    match conn {
        Ok(conn) => {
            let count = inventory_for(name, handle).await.map(|inv| inv.tools.len());
            match count {
                Ok(count) => {
                    adopt_connected(state, &cfg, conn);
                    Ok(format!("mcp `{name}` on ({count} tools)"))
                }
                Err(e) => {
                    adopt_failed(state, &cfg, e.clone());
                    Err(e)
                }
            }
        }
        Err(e) => {
            adopt_failed(state, &cfg, e.clone());
            Err(e)
        }
    }
}

/// Spawn a stdio MCP server, list its tools, and register one
/// `mcp__<server>__<tool>` runtime-defined tool per remote tool on the
/// shared handle. Returns the connection (hold it; dropping retires
/// the tools). Spawn / connect / list failures are `Err` with a
/// human-readable reason — the caller warns and continues.
pub(crate) async fn connect_stdio(
    cfg: &ServerConfig,
    handle: &rig::agent::tool::server::ToolServerHandle,
    mcp: &SharedMcpState,
) -> Result<Connection, String> {
    use process_wrap::tokio::CommandWrap;
    let (command, args, env) = match &cfg.transport {
        Transport::Stdio { command, args, env } => (command, args, env),
        other => return Err(format!("mcp `{}`: not a stdio entry ({other:?})", cfg.name)),
    };
    let wrap = CommandWrap::with_new(command, |cmd| {
        cmd.args(args);
        cmd.envs(env);
    });
    let transport = rmcp::transport::TokioChildProcess::new(wrap)
        .map_err(|e| format!("mcp `{}`: spawn `{command}` failed: {e}", cfg.name))?;
    let service = RefreshHandler::new(cfg.name.clone(), handle.clone(), mcp.clone())
        .serve(transport)
        .await?;
    let peer = RmcpPeer(service.peer().clone());
    // Unreachable: serve() already listed + registered. Kept as a
    // fail-closed re-check so a silent serve bug can't hide tools.
    let tools = peer
        .list_tools()
        .await
        .map_err(|e| format!("mcp `{}`: list_tools failed: {e}", cfg.name))?;
    if tools.is_empty() {
        return Err(format!("mcp `{}`: server listed zero tools", cfg.name));
    }
    Ok(Connection { _service: service })
}

/// Connect one streamable-HTTP MCP server onto the shared handle.
/// Same contract as [`connect_stdio`]: prefixed registration +
/// held connection, `Err` with reason for warn-and-continue.
/// Static `headers` ride on every request (Ticket 06 adds OAuth).
/// Outcome of one HTTP connect attempt. `NeedsAuth` is distinct from
/// `Err`: the server is reachable but OAuth login is required — adopt
/// a needs-auth record (login hint) rather than a failure.
enum HttpConnectOutcome {
    Connected(Connection),
    NeedsAuth { hint: String },
}

pub(crate) async fn connect_http(
    cfg: &ServerConfig,
    handle: &rig::agent::tool::server::ToolServerHandle,
    mcp: &SharedMcpState,
) -> Result<Connection, String> {
    let store = FileTokenStore::new(tokens_path().map_err(|e| format!("mcp `{}`: {e}", cfg.name))?);
    connect_http_with_store(cfg, handle, mcp, &store).await
}

/// HTTP connect with an explicit token store (tests inject a temp
/// path; production passes the default `tokens_path()` store).
pub(crate) async fn connect_http_with_store(
    cfg: &ServerConfig,
    handle: &rig::agent::tool::server::ToolServerHandle,
    mcp: &SharedMcpState,
    store: &FileTokenStore,
) -> Result<Connection, String> {
    match connect_http_inner(cfg, handle, mcp, store).await {
        Ok(HttpConnectOutcome::Connected(conn)) => Ok(conn),
        Ok(HttpConnectOutcome::NeedsAuth { hint }) => Err(hint),
        Err(e) => Err(e),
    }
}

/// Inner HTTP connect with a typed needs-auth outcome for `connect_all`.
async fn connect_http_inner(
    cfg: &ServerConfig,
    handle: &rig::agent::tool::server::ToolServerHandle,
    mcp: &SharedMcpState,
    store: &FileTokenStore,
) -> Result<HttpConnectOutcome, String> {
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    let (url, headers, auth) = match &cfg.transport {
        Transport::StreamableHttp { url, headers, auth } => (url, headers, auth),
        other => return Err(format!("mcp `{}`: not an HTTP entry ({other:?})", cfg.name)),
    };
    // OAuth opt-in: no stored token marks needs-auth (login hint)
    // without touching the network. A stored token wires AuthClient
    // below so the bearer rides every request (incl. list_tools).
    let stored_token = match *auth {
        true => match store.load_sync(&cfg.name) {
            Some(t) => Some(t),
            None => {
                return Ok(HttpConnectOutcome::NeedsAuth {
                    hint: format!(
                        "mcp `{}`: OAuth login required — run /mcp login {}",
                        cfg.name, cfg.name
                    ),
                });
            }
        },
        false => None,
    };
    let mut custom_headers = std::collections::HashMap::new();
    for (k, v) in headers {
        let name: http::HeaderName = k
            .parse()
            .map_err(|_| format!("mcp `{}`: bad header name `{k}`", cfg.name))?;
        let value: http::HeaderValue = v
            .parse()
            .map_err(|_| format!("mcp `{}`: bad header value for `{k}`", cfg.name))?;
        custom_headers.insert(name, value);
    }
    let config =
        StreamableHttpClientTransportConfig::with_uri(url.as_str()).custom_headers(custom_headers);
    // Stored-token path: wrap the inner HTTP client in AuthClient so the
    // bearer rides every request (initialize, list, call). Plain path
    // keeps the direct transport (static headers only).
    let service = match stored_token {
        Some(token) => {
            let manager = oauth_manager_for_stored(url, &cfg.name, store, &token).await?;
            let inner = reqwest013::Client::builder()
                .pool_max_idle_per_host(0)
                .redirect(reqwest013::redirect::Policy::none())
                .build()
                .map_err(|e| format!("mcp `{}`: http client failed: {e}", cfg.name))?;
            let auth_client = rmcp::transport::auth::AuthClient::new(inner, manager);
            let transport =
                rmcp::transport::StreamableHttpClientTransport::with_client(auth_client, config);
            RefreshHandler::with_store(cfg.name.clone(), handle.clone(), mcp.clone(), store.clone())
                .serve(transport)
                .await?
        }
        None => {
            let transport = rmcp::transport::StreamableHttpClientTransport::from_config(config);
            RefreshHandler::with_store(cfg.name.clone(), handle.clone(), mcp.clone(), store.clone())
                .serve(transport)
                .await?
        }
    };
    let peer = RmcpPeer(service.peer().clone());
    let tools = peer
        .list_tools()
        .await
        .map_err(|e| format!("mcp `{}`: list_tools failed: {e}", cfg.name))?;
    if tools.is_empty() {
        return Err(format!("mcp `{}`: server listed zero tools", cfg.name));
    }
    Ok(HttpConnectOutcome::Connected(Connection {
        _service: service,
    }))
}

/// Build an `AuthorizationManager` seeded from the file token store
/// for one server. The bridge feeds the stored access token through
/// rmcp's `CredentialStore`; no-expiry tokens return with zero
/// network (manager fast path), so plain bearer servers connect
/// without discovery round-trips.
async fn oauth_manager_for_stored(
    url: &str,
    server: &str,
    store: &FileTokenStore,
    _token: &StoredMcpToken,
) -> Result<rmcp::transport::auth::AuthorizationManager, String> {
    let mut manager = rmcp::transport::auth::AuthorizationManager::new(url)
        .await
        .map_err(|e| format!("mcp `{server}`: oauth setup failed: {e}"))?;
    manager.set_credential_store(StoreCredentialBridge {
        store: store.clone(),
        server: server.to_string(),
    });
    Ok(manager)
}

/// One connected server's tool inventory for the preamble index:
/// sanitized server name plus bare tool names + descriptions.
#[derive(Debug, Clone)]
pub(crate) struct ServerInventory {
    pub(crate) server: String,
    pub(crate) tools: Vec<ToolInventory>,
}

/// One tool's preamble entry.
#[derive(Debug, Clone)]
pub(crate) struct ToolInventory {
    pub(crate) bare_name: String,
    pub(crate) description: String,
}

/// Outcome of connecting every configured server: warning strings
/// (config warnings + per-server failures) and tool inventory for the
/// preamble index. Held connections live in the shared records, not
/// here — dropping the report must not retire tools.
#[derive(Default)]
pub(crate) struct ConnectReport {
    pub(crate) warnings: Vec<String>,
    pub(crate) inventory: Vec<ServerInventory>,
}

/// Begin OAuth login for one configured server: dynamic client
/// registration + localhost-callback URL. Returns the URL to open
/// (the caller prints it and tries the system browser). The browser
/// redirect lands on the local callback listener; `mcp_login_callback`
/// completes the exchange. Headless stays printable-URL-first.
pub(crate) async fn mcp_login(
    state: &SharedMcpState,
    handle: &rig::agent::tool::server::ToolServerHandle,
    name: &str,
    store: &FileTokenStore,
) -> Result<String, String> {
    use rmcp::transport::auth::{AuthorizationManager, AuthorizationSession};
    let (url, transport_kind) = {
        let records = state
            .lock()
            .map_err(|_| "mcp state lock poisoned".to_string())?;
        match records.iter().find(|r| r.name == name) {
            Some(r) => match &r.transport {
                Transport::StreamableHttp { url, .. } => (url.clone(), "http"),
                Transport::Stdio { .. } => {
                    return Err(format!(
                        "mcp `{name}`: login is HTTP OAuth only (stdio needs no login)"
                    ));
                }
            },
            None => return Err(format!("no MCP server `{name}`")),
        }
    };
    let _ = transport_kind;
    // Localhost callback on an ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("mcp `{name}`: callback bind failed: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("mcp `{name}`: callback addr failed: {e}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let mut manager = AuthorizationManager::new(url.as_str())
        .await
        .map_err(|e| format!("mcp `{name}`: oauth setup failed: {e}"))?;
    manager.set_credential_store(StoreCredentialBridge {
        store: store.clone(),
        server: name.to_string(),
    });
    let metadata = manager
        .discover_metadata()
        .await
        .map_err(|e| format!("mcp `{name}`: oauth discovery failed: {e}"))?;
    manager.set_metadata(metadata);
    let session = AuthorizationSession::new(manager, &[], &redirect_uri, Some("rem"), None)
        .await
        .map_err(|e| format!("mcp `{name}`: registration failed: {e}"))?;
    let auth_url = session.get_authorization_url().to_string();
    // Spawn the callback listener: single redirect → exchange →
    // reconnect, all inside this task. The auth URL print + browser
    // open above are the user-visible contract; completion surfaces
    // as tools appearing + a transcript notice on next poll.
    let server_name = name.to_string();
    let handle_clone = handle.clone();
    let state_clone: SharedMcpState = state.clone();
    let store_clone = store.clone();
    tokio::spawn(async move {
        if let Ok(callback_url) = wait_for_oauth_callback(listener).await {
            let _ = mcp_login_callback(
                &state_clone,
                &handle_clone,
                &server_name,
                &store_clone,
                session,
                &callback_url,
            )
            .await;
        }
    });
    // Best-effort browser open; the printed URL is the contract
    // (SSH/headless copy-paste).
    open_browser(&auth_url);
    Ok(auth_url)
}

/// Attempt to open a URL in the system browser. Best-effort: never
/// fails the login — the printed URL is the contract.
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let _ = url;
}

/// Wait for one OAuth redirect on the localhost listener. Returns the
/// full callback URL (`?code=…&state=…`). Times out after 5 minutes
/// so a forgotten browser tab can't hold the task forever.
async fn wait_for_oauth_callback(listener: tokio::net::TcpListener) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(300), listener.accept())
            .await
            .map_err(|_| "mcp login: timed out waiting for browser callback".to_string())?
            .map_err(|e| format!("mcp login: callback accept failed: {e}"))?;
    let mut buf = vec![0u8; 8192];
    let n = tokio::time::timeout(std::time::Duration::from_secs(30), stream.read(&mut buf))
        .await
        .map_err(|_| "mcp login: timed out reading callback".to_string())?
        .map_err(|e| format!("mcp login: callback read failed: {e}"))?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .ok_or_else(|| "mcp login: bad callback request".to_string())?;
    use tokio::io::AsyncWriteExt;
    let _ = stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nConnection: close\r\n\r\nLogin complete")
        .await;
    // Reconstruct a full URL for `handle_callback_url`.
    Ok(format!("http://127.0.0.1{path}"))
}

/// Complete the OAuth exchange after the browser redirect: code →
/// tokens → file store → reconnect + re-register. Returns a human
/// summary for the `/mcp` notice.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn mcp_login_callback(
    state: &SharedMcpState,
    handle: &rig::agent::tool::server::ToolServerHandle,
    name: &str,
    store: &FileTokenStore,
    session: rmcp::transport::auth::AuthorizationSession,
    callback_url: &str,
) -> Result<String, String> {
    let token_response = session
        .handle_callback_url(callback_url)
        .await
        .map_err(|e| format!("mcp `{name}`: token exchange failed: {e}"))?;
    // Stamp the token row: access + refresh (when issued).
    {
        use oauth2::TokenResponse;
        let refresh = token_response
            .refresh_token()
            .map(|t| t.secret().to_string());
        store
            .save_full_sync(
                name,
                StoredMcpToken {
                    access_token: token_response.access_token().secret().to_string(),
                    refresh_token: refresh,
                    scopes: vec![],
                },
            )
            .map_err(|e| format!("mcp `{name}`: {e}"))?;
    }
    // Reconnect through the recorded transport with the fresh token.
    let transport = {
        let records = state
            .lock()
            .map_err(|_| "mcp state lock poisoned".to_string())?;
        match records.iter().find(|r| r.name == name) {
            Some(r) => r.transport.clone(),
            None => return Err(format!("no MCP server `{name}`")),
        }
    };
    // Flip enabled first so serve() registers (same rule as toggle-on).
    if let Ok(mut records) = state.lock()
        && let Some(r) = records.iter_mut().find(|r| r.name == name)
    {
        r.enabled = true;
    }
    let cfg = ServerConfig {
        name: name.to_string(),
        transport,
    };
    let conn = match &cfg.transport {
        Transport::Stdio { .. } => connect_stdio(&cfg, handle, state).await,
        Transport::StreamableHttp { .. } => {
            connect_http_with_store(&cfg, handle, state, store).await
        }
    };
    match conn {
        Ok(conn) => {
            let count = inventory_for(name, handle)
                .await
                .map(|inv| inv.tools.len())
                .unwrap_or(0);
            adopt_connected(state, &cfg, conn);
            Ok(format!("mcp `{name}` logged in ({count} tools)"))
        }
        Err(e) => {
            adopt_failed(state, &cfg, e.clone());
            Err(e)
        }
    }
}

/// Log out of one server: clear the stored token, drop the connection,
/// remove its tools. Returns a human summary for the `/mcp` notice.
pub(crate) async fn mcp_logout(
    state: &SharedMcpState,
    handle: &rig::agent::tool::server::ToolServerHandle,
    name: &str,
    store: &FileTokenStore,
) -> Result<String, String> {
    store.clear_sync(name);
    // Reuse the toggle-off path: drop connection + remove tools.
    mcp_set_enabled(state, handle, name, false).await?;
    // Mark the record needs-auth so the panel hints at login.
    if let Ok(mut records) = state.lock()
        && let Some(r) = records.iter_mut().find(|r| r.name == name)
    {
        r.status = McpConnStatus::NeedsAuth;
    }
    Ok(format!("mcp `{name}` logged out"))
}

/// Printable login hints for every needs-auth server in the shared
/// records, for startup auto-login display. Pure projection — no I/O.
/// Returns one line per needs-auth server, sorted by name.
pub(crate) fn needs_auth_login_hints(names: &[String]) -> Vec<String> {
    let mut names: Vec<String> = names.to_vec();
    names.sort();
    names
        .into_iter()
        .map(|n| format!("mcp `{n}`: OAuth login required — run /mcp login {n}"))
        .collect()
}

/// Names of session-enabled needs-auth servers in the shared records,
/// for startup auto-login display. Sorted for stable output.
pub(crate) fn needs_auth_servers(state: &SharedMcpState) -> Vec<String> {
    let mut out: Vec<String> = state
        .lock()
        .map(|records| {
            records
                .iter()
                .filter(|r| r.enabled && matches!(r.status, McpConnStatus::NeedsAuth))
                .map(|r| r.name.clone())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Panel listing for `/mcp`: one row per known server with live tool
pub(crate) async fn mcp_server_summaries(
    state: &SharedMcpState,
    handle: &rig::agent::tool::server::ToolServerHandle,
) -> Vec<crate::agent::McpServerSummary> {
    use crate::agent::{McpServerStatus, McpServerSummary};
    let snapshot: Vec<(String, McpConnStatus, bool)> = state
        .lock()
        .map(|records| {
            records
                .iter()
                .map(|r| (r.name.clone(), r.status.clone(), r.enabled))
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::with_capacity(snapshot.len());
    for (name, status, enabled) in snapshot {
        let (status, tool_count) = match status {
            McpConnStatus::Connected => {
                let count = inventory_for(&name, handle)
                    .await
                    .map(|inv| inv.tools.len())
                    .unwrap_or(0);
                (McpServerStatus::Connected, count)
            }
            McpConnStatus::Failed { reason } => (
                McpServerStatus::Failed {
                    reason: reason.clone(),
                },
                0,
            ),
            McpConnStatus::NeedsAuth => (McpServerStatus::NeedsAuth, 0),
        };
        out.push(McpServerSummary {
            name,
            status,
            tool_count,
            enabled,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Inventory for every session-enabled connected server, for preamble
/// rebuilds after `/mcp` toggles. Sorted for a stable preamble.
pub(crate) async fn inventory_for_enabled(
    state: &SharedMcpState,
    handle: &rig::agent::tool::server::ToolServerHandle,
) -> Vec<ServerInventory> {
    let enabled: Vec<String> = state
        .lock()
        .map(|records| {
            records
                .iter()
                .filter(|r| r.enabled && matches!(r.status, McpConnStatus::Connected))
                .map(|r| r.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::with_capacity(enabled.len());
    for name in &enabled {
        if let Ok(inv) = inventory_for(name, handle).await {
            out.push(inv);
        }
    }
    out.sort_by(|a, b| a.server.cmp(&b.server));
    out
}

/// Read one server's tool inventory off the shared handle by prefix:
/// `mcp__<server>__*` defs, stripped back to bare names.
async fn inventory_for(
    server: &str,
    handle: &rig::agent::tool::server::ToolServerHandle,
) -> Result<ServerInventory, String> {
    let prefix = format!("mcp__{}__", sanitize_server_name(server));
    let defs = handle
        .get_tool_defs(None)
        .await
        .map_err(|e| format!("mcp `{server}`: tool defs failed: {e}"))?;
    let mut tools: Vec<ToolInventory> = defs
        .into_iter()
        .filter_map(|d| {
            d.name.strip_prefix(&prefix).map(|bare| ToolInventory {
                bare_name: bare.to_string(),
                description: d.description,
            })
        })
        .collect();
    tools.sort_by(|a, b| a.bare_name.cmp(&b.bare_name));
    Ok(ServerInventory {
        server: server.to_string(),
        tools,
    })
}

/// Connect every configured server onto the shared handle.
/// Returns held connections plus per-server warning strings.
/// Warn-and-continue: one broken server never fails the batch.
pub(crate) async fn connect_all(
    loaded: &LoadedConfig,
    handle: &rig::agent::tool::server::ToolServerHandle,
    mcp: &SharedMcpState,
) -> ConnectReport {
    let mut report = ConnectReport {
        warnings: loaded.warnings.clone(),
        ..Default::default()
    };
    // Default token store for production connects; tests inject temp
    // paths through `connect_http_with_store` directly. A missing
    // config dir degrades to needs-auth rather than failing startup.
    let default_store = FileTokenStore::new(
        tokens_path().unwrap_or_else(|_| std::path::PathBuf::from("mcp-tokens.json")),
    );
    for cfg in &loaded.servers {
        // HTTP goes through the inner typed outcome so needs-auth adopts
        // a login-hint record instead of a failure; stdio has no auth.
        if matches!(cfg.transport, Transport::StreamableHttp { .. }) {
            match connect_http_inner(cfg, handle, mcp, &default_store).await {
                Ok(HttpConnectOutcome::Connected(conn)) => {
                    match inventory_for(&cfg.name, handle).await {
                        Ok(inv) => {
                            adopt_connected(mcp, cfg, conn);
                            report.inventory.push(inv);
                        }
                        Err(e) => {
                            adopt_failed(mcp, cfg, e.clone());
                            report.warnings.push(e);
                        }
                    }
                }
                Ok(HttpConnectOutcome::NeedsAuth { hint }) => {
                    adopt_needs_auth(mcp, cfg);
                    report.warnings.push(hint);
                }
                Err(e) => {
                    adopt_failed(mcp, cfg, e.clone());
                    report.warnings.push(e);
                }
            }
            continue;
        }
        let outcome = connect_stdio(cfg, handle, mcp).await;
        match outcome {
            Ok(conn) => match inventory_for(&cfg.name, handle).await {
                Ok(inv) => {
                    adopt_connected(mcp, cfg, conn);
                    report.inventory.push(inv);
                }
                Err(e) => {
                    adopt_failed(mcp, cfg, e.clone());
                    report.warnings.push(e);
                }
            },
            Err(e) => {
                adopt_failed(mcp, cfg, e.clone());
                report.warnings.push(e);
            }
        }
    }
    // Sorted for a stable preamble.
    report.inventory.sort_by(|a, b| a.server.cmp(&b.server));
    report
}

/// How to reach one MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Transport {
    /// Local command over stdio: expanded command + args + env.
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    /// Remote URL over streamable HTTP: expanded url + headers.
    /// `auth: true` opts into MCP OAuth 2.0 (Ticket 06 slice 1:
    /// stored token or needs-auth; slice 2 adds the login flow).
    StreamableHttp {
        url: String,
        headers: HashMap<String, String>,
        #[allow(dead_code)]
        auth: bool,
    },
}

/// One configured MCP server: sanitized name + transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerConfig {
    pub(crate) name: String,
    pub(crate) transport: Transport,
}

/// Loaded config: valid servers plus per-entry warnings.
/// (Ticket 03 reads `servers`; Ticket 02 surfaces `warnings`.)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedConfig {
    #[allow(dead_code)]
    pub(crate) servers: Vec<ServerConfig>,
    pub warnings: Vec<String>,
}

/// Expand `${VAR}` / `${VAR:-default}` in one string. Unset without
/// default expands to empty.
fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            None => {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            }
            Some(end) => {
                let expr = &after[..end];
                let (var, default) = match expr.split_once(":-") {
                    Some((v, d)) => (v, Some(d)),
                    None => (expr, None),
                };
                match std::env::var(var) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        if let Some(d) = default {
                            out.push_str(d);
                        }
                    }
                }
                rest = &after[end + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parse a `mcpServers`-shaped JSON value. Invalid entries warn and
/// skip; a `url` key selects streamable-HTTP, a `command` key selects
/// stdio. Never fails on bad entries.
fn parse_config(json: &serde_json::Value) -> LoadedConfig {
    let mut loaded = LoadedConfig::default();
    let servers = json.get("mcpServers").and_then(|v| v.as_object());
    let Some(servers) = servers else {
        return loaded;
    };
    // Sorted for deterministic order in tests and startup.
    let mut names: Vec<&String> = servers.keys().collect();
    names.sort();
    for name in names {
        let entry = &servers[name];
        let obj = entry.as_object();
        let transport = match obj {
            Some(o) if o.contains_key("url") => {
                let url = expand_env(o.get("url").and_then(|v| v.as_str()).unwrap_or(""));
                if url.is_empty() {
                    None
                } else {
                    let headers = o
                        .get("headers")
                        .and_then(|v| v.as_object())
                        .map(|h| {
                            h.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), expand_env(s))))
                                .collect()
                        })
                        .unwrap_or_default();
                    let auth = o.get("auth").and_then(|v| v.as_bool()).unwrap_or(false);
                    Some(Transport::StreamableHttp { url, headers, auth })
                }
            }
            Some(o) if o.contains_key("command") => {
                let command = expand_env(o.get("command").and_then(|v| v.as_str()).unwrap_or(""));
                if command.is_empty() {
                    None
                } else {
                    let args = o
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(expand_env))
                                .collect()
                        })
                        .unwrap_or_default();
                    let env = o
                        .get("env")
                        .and_then(|v| v.as_object())
                        .map(|e| {
                            e.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), expand_env(s))))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(Transport::Stdio { command, args, env })
                }
            }
            _ => None,
        };
        match transport {
            Some(transport) => loaded.servers.push(ServerConfig {
                name: sanitize_server_name(name),
                transport,
            }),
            None => loaded
                .warnings
                .push(format!("mcp: skipping invalid entry `{name}`")),
        }
    }
    loaded
}

/// Load from one path. Missing file = zero servers, no error, no
/// warnings. Invalid JSON = one warning, zero servers.
fn load_from_path(path: &Path) -> LoadedConfig {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadedConfig::default(),
        Err(e) => {
            return LoadedConfig {
                warnings: vec![format!("mcp: cannot read {}: {e}", path.display())],
                ..Default::default()
            };
        }
    };
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(json) => parse_config(&json),
        Err(e) => LoadedConfig {
            warnings: vec![format!("mcp: invalid JSON in {}: {e}", path.display())],
            ..Default::default()
        },
    }
}

/// Sanitize an MCP server name to `[A-Za-z0-9_-]` for
/// `mcp__<server>__<tool>` names (Claude convention). Anything else
/// becomes `_` so odd folder / URL names can't produce ambiguous tools.
fn sanitize_server_name(name: &str) -> String {
    name.chars()
        .map(
            |c| match c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                true => c,
                false => '_',
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn real_peer_lists_and_calls_over_the_wire() {
        let (client, task) = fake_running_client().await;
        let peer = RmcpPeer(client.peer().clone());
        let defs = peer.list_tools().await.expect("list_tools over duplex");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "create_issue");
        let out = peer
            .call_tool(
                "create_issue".to_string(),
                serde_json::json!({"title": "bug"}),
            )
            .await
            .expect("call_tool over duplex");
        assert!(out.contains("wire:create_issue:bug"), "got: {out}");
        task.abort();
    }

    /// Minimal MCP server over stdio, dependency-free: speaks
    /// initialize + tools/list + tools/call as line-delimited JSON-RPC.
    const PY_MCP_SERVER: &str = r#"
import json, sys
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except Exception:
        continue
    mid = msg.get("id")
    method = msg.get("method", "")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "rem-py-test", "version": "0.1"}}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": mid, "result": {"tools": [{
            "name": "echo_downcase",
            "description": "Echo the text lowercased",
            "inputSchema": {"type": "object",
                "properties": {"text": {"type": "string"}}},
            "annotations": {"title": "downcaser"}}]}})
    elif method == "tools/call":
        text = (msg.get("params") or {}).get("arguments", {}).get("text", "?")
        send({"jsonrpc": "2.0", "id": mid, "result": {"content": [
            {"type": "text", "text": "py:" + str(text).lower()}]}})
    else:
        send({"jsonrpc": "2.0", "id": mid, "error": {
            "code": -32601, "message": "unknown: " + method}})
"#;

    #[tokio::test]
    async fn connect_stdio_spawns_lists_and_registers_prefixed_tools() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::ToolContext;
        let script = std::env::temp_dir().join("rem-mcp-py-server-test.py");
        std::fs::write(&script, PY_MCP_SERVER).unwrap();
        let cfg = ServerConfig {
            name: "py".to_string(),
            transport: Transport::Stdio {
                command: "python3".to_string(),
                args: vec![script.to_string_lossy().to_string()],
                env: Default::default(),
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let conn = connect_stdio(&cfg, &handle, &state)
            .await
            .expect("spawn + list");
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("tool defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["mcp__py__echo_downcase".to_string()]);
        let mut ctx = ToolContext::new();
        let result = handle
            .execute("mcp__py__echo_downcase", r#"{"text":"HeLLo"}"#, &mut ctx)
            .await;
        assert!(result.is_success(), "{result:?}");
        assert!(
            result.output().render().contains("py:hello"),
            "got: {}",
            result.output().render()
        );
        drop(conn);
        let _ = std::fs::remove_file(&script);
    }

    #[tokio::test]
    async fn connect_stdio_failure_names_the_server() {
        use rig::agent::tool::server::ToolServer;
        let cfg = ServerConfig {
            name: "broken".to_string(),
            transport: Transport::Stdio {
                command: "/nonexistent/rem-mcp-test-binary-xyz".to_string(),
                args: vec![],
                env: Default::default(),
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let err = match connect_stdio(&cfg, &handle, &state).await {
            Ok(_) => panic!("bad binary connected"),
            Err(e) => e,
        };
        assert!(err.contains("broken"), "got: {err}");
        // Nothing registered: session proceeds on built-ins.
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("tool defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(names.is_empty(), "got: {names:?}");
    }

    #[tokio::test]
    async fn connect_all_warns_and_continues_past_broken_servers() {
        use rig::agent::tool::server::ToolServer;
        let script = std::env::temp_dir().join("rem-mcp-py-server-connect-all-test.py");
        std::fs::write(&script, PY_MCP_SERVER).unwrap();
        let loaded = LoadedConfig {
            servers: vec![
                ServerConfig {
                    name: "py".to_string(),
                    transport: Transport::Stdio {
                        command: "python3".to_string(),
                        args: vec![script.to_string_lossy().to_string()],
                        env: Default::default(),
                    },
                },
                ServerConfig {
                    name: "broken".to_string(),
                    transport: Transport::Stdio {
                        command: "/nonexistent/rem-mcp-test-binary-xyz".to_string(),
                        args: vec![],
                        env: Default::default(),
                    },
                },
            ],
            warnings: vec![],
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let report = connect_all(&loaded, &handle, &state).await;
        // Broken server warns by name; good server still registers.
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].contains("broken"),
            "got: {:?}",
            report.warnings
        );
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("tool defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["mcp__py__echo_downcase".to_string()]);
        let _ = std::fs::remove_file(&script);
    }

    #[tokio::test]
    async fn server_instructions_prepend_to_tool_descriptions() {
        use rig::agent::tool::server::ToolServer;
        let server = FakeServer::with_instructions("Always downcase the text.");
        let handle = ToolServer::new().run();
        let (c2s, sfc) = tokio::io::duplex(8192);
        let (s2c, cfs) = tokio::io::duplex(8192);
        tokio::spawn(async move {
            use rmcp::ServiceExt;
            let running = server.serve((sfc, s2c)).await.expect("server start");
            let _ = running.waiting().await;
        });
        let client = RefreshHandler::test_handler("py", &handle)
            .serve((cfs, c2s))
            .await
            .expect("client connect");
        let instructions = client
            .peer_info()
            .as_ref()
            .and_then(|info| info.instructions.clone());
        reconcile_server_tools("py", &handle, client.peer(), instructions.as_deref())
            .await
            .expect("register");
        let defs = handle.get_tool_defs(None).await.expect("defs");
        assert_eq!(defs.len(), 1);
        assert!(
            defs[0].description.contains("Always downcase the text."),
            "got: {}",
            defs[0].description
        );
        client.cancel().await.unwrap();
    }

    #[test]
    fn preamble_section_lists_prefixed_tools_per_server() {
        let inventory = vec![ServerInventory {
            server: "py".to_string(),
            tools: vec![ToolInventory {
                bare_name: "echo_downcase".to_string(),
                description: "Echo the text lowercased".to_string(),
            }],
        }];
        let section = preamble_section(&inventory);
        assert!(section.contains("mcp__py__echo_downcase"), "got: {section}");
        assert!(
            section.contains("Echo the text lowercased"),
            "got: {section}"
        );
        assert!(preamble_section(&[]).is_empty());
    }

    #[tokio::test]
    async fn connect_all_reports_inventory_for_preamble() {
        use rig::agent::tool::server::ToolServer;
        let script = std::env::temp_dir().join("rem-mcp-py-inventory-test.py");
        std::fs::write(&script, PY_MCP_SERVER).unwrap();
        let loaded = LoadedConfig {
            servers: vec![ServerConfig {
                name: "py".to_string(),
                transport: Transport::Stdio {
                    command: "python3".to_string(),
                    args: vec![script.to_string_lossy().to_string()],
                    env: Default::default(),
                },
            }],
            warnings: vec!["earlier warning".to_string()],
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let report = connect_all(&loaded, &handle, &state).await;
        // Earlier warnings flow through; inventory groups by server with
        // bare names + descriptions for the preamble index.
        assert_eq!(report.warnings, vec!["earlier warning".to_string()]);
        assert_eq!(report.inventory.len(), 1);
        assert_eq!(report.inventory[0].server, "py");
        assert_eq!(report.inventory[0].tools.len(), 1);
        assert_eq!(report.inventory[0].tools[0].bare_name, "echo_downcase");
        assert!(
            !report.inventory[0].tools[0].description.is_empty(),
            "description missing"
        );
        let _ = std::fs::remove_file(&script);
    }

    #[tokio::test]
    async fn reconcile_registers_prefixed_names_and_dispatches_over_wire() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::ToolContext;
        let (client, task) = fake_running_client().await;
        let handle = ToolServer::new().run();
        reconcile_server_tools("github", &handle, client.peer(), None)
            .await
            .expect("reconcile");
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("tool defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["mcp__github__create_issue".to_string()]);
        let mut ctx = ToolContext::new();
        let result = handle
            .execute("mcp__github__create_issue", r#"{"title":"bug"}"#, &mut ctx)
            .await;
        assert!(result.is_success(), "{result:?}");
        let rendered = result.output().render();
        assert!(
            rendered.contains("wire:create_issue:bug"),
            "got: {rendered}"
        );
        task.abort();
    }

    /// Fake server with a mutable tool list for refresh tests.
    #[derive(Clone)]
    struct ChangingServer {
        tools: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
    }

    impl rmcp::ServerHandler for ChangingServer {
        fn get_info(&self) -> rmcp::model::ServerInfo {
            rmcp::model::ServerInfo::new(
                rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
            )
            .with_protocol_version(rmcp::model::ProtocolVersion::LATEST)
            .with_server_info(rmcp::model::Implementation::new(
                "rem-mcp-changing",
                "0.1.0",
            ))
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::ListToolsResult, rmcp::model::ErrorData> {
            use std::sync::Arc;
            let names = self.tools.lock().await.clone();
            Ok(rmcp::model::ListToolsResult {
                tools: names
                    .into_iter()
                    .map(|n| {
                        rmcp::model::Tool::new(n, "changing tool", Arc::new(serde_json::Map::new()))
                    })
                    .collect(),
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            request: rmcp::model::CallToolRequestParams,
            _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(format!("changing:{}", request.name)),
            ]))
        }
    }

    #[tokio::test]
    async fn refresh_on_list_changed_replaces_servers_tools_only() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::ToolContext;
        use rmcp::ServiceExt;
        let tools = std::sync::Arc::new(tokio::sync::Mutex::new(vec!["alpha".to_string()]));
        let server = ChangingServer {
            tools: tools.clone(),
        };
        let handle = ToolServer::new().run();
        let (c2s, sfc) = tokio::io::duplex(8192);
        let (s2c, cfs) = tokio::io::duplex(8192);
        let server_task =
            tokio::spawn(async move { server.serve((sfc, s2c)).await.expect("server start") });
        let client = RefreshHandler::test_handler("one", &handle)
            .serve((cfs, c2s))
            .await
            .expect("client connect");
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["mcp__one__alpha".to_string()]);
        // Other-server tool present before refresh must survive it.
        handle
            .add_dynamic_tool(rig::tool::DynamicTool::new(
                "mcp__two__kept",
                "other server",
                serde_json::json!({"type": "object"}),
                |_ctx, _args| Box::pin(async move { Ok(rig::tool::ToolOutput::text("kept")) }),
            ))
            .await;
        // Server changes its list, then notifies over the real wire.
        {
            let mut guard = tools.lock().await;
            *guard = vec!["beta".to_string(), "gamma".to_string()];
        }
        let running_server = server_task.await.expect("server running");
        running_server
            .peer()
            .notify_tool_list_changed()
            .await
            .expect("notify");
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let names: Vec<String> = handle
                    .get_tool_defs(None)
                    .await
                    .expect("defs")
                    .into_iter()
                    .map(|d| d.name)
                    .collect();
                if names.contains(&"mcp__one__beta".to_string()) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("refresh");
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(
            names.contains(&"mcp__one__beta".to_string()),
            "got: {names:?}"
        );
        assert!(
            names.contains(&"mcp__one__gamma".to_string()),
            "got: {names:?}"
        );
        assert!(
            !names.contains(&"mcp__one__alpha".to_string()),
            "got: {names:?}"
        );
        assert!(
            names.contains(&"mcp__two__kept".to_string()),
            "got: {names:?}"
        );
        // Refreshed tools dispatch under bare names.
        let mut ctx = ToolContext::new();
        let result = handle.execute("mcp__one__beta", r#"{}"#, &mut ctx).await;
        assert!(result.is_success(), "{result:?}");
        assert!(
            result.output().render().contains("changing:beta"),
            "got: {}",
            result.output().render()
        );
        client.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn dropped_server_tools_retire_with_replannable_error() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::{ToolContext, ToolErrorKind};
        let tools = std::sync::Arc::new(tokio::sync::Mutex::new(vec!["alpha".to_string()]));
        let server = ChangingServer { tools };
        let handle = ToolServer::new().run();
        let (c2s, sfc) = tokio::io::duplex(8192);
        let (s2c, cfs) = tokio::io::duplex(8192);
        let server_task = tokio::spawn(async move {
            use rmcp::ServiceExt;
            server.serve((sfc, s2c)).await.expect("server start")
        });
        let client = RefreshHandler::test_handler("one", &handle)
            .serve((cfs, c2s))
            .await
            .expect("client connect");
        assert_eq!(handle.get_tool_defs(None).await.expect("defs").len(), 1);
        // Kill the server side: transport closes, tools must retire and
        // the model gets a replannable error naming the tool.
        server_task.abort();
        client.cancel().await.unwrap();
        // Retirement triggers on dispatch (the self-retire check runs in
        // the tool callback, not on def listing). One attempt retires;
        // the attempt itself fails replannable.
        let result = handle
            .execute("mcp__one__alpha", r#"{}"#, &mut ToolContext::new())
            .await;
        assert!(
            result.is_error_kind(ToolErrorKind::NotFound),
            "dropped tool must fail replannable, got: {result:?}"
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let defs = handle.get_tool_defs(None).await.expect("defs");
                if defs.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retire");
        let result = handle
            .execute("mcp__one__alpha", r#"{}"#, &mut ToolContext::new())
            .await;
        assert!(
            result.is_error_kind(ToolErrorKind::NotFound),
            "dropped tool must fail replannable, got: {result:?}"
        );
    }

    /// HTTP fake server: same tool shape as FakeServer but answers with
    /// an `http:` prefix so tests prove which transport served the call.
    #[derive(Clone)]
    struct HttpToolServer;

    impl rmcp::ServerHandler for HttpToolServer {
        fn get_info(&self) -> rmcp::model::ServerInfo {
            rmcp::model::ServerInfo::new(
                rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
            )
            .with_protocol_version(rmcp::model::ProtocolVersion::LATEST)
            .with_server_info(rmcp::model::Implementation::new(
                "rem-mcp-http-test",
                "0.1.0",
            ))
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::ListToolsResult, rmcp::model::ErrorData> {
            use std::sync::Arc;
            Ok(rmcp::model::ListToolsResult {
                tools: vec![rmcp::model::Tool::new(
                    "http_echo",
                    "Echo the text lowercased over HTTP",
                    Arc::new(serde_json::Map::new()),
                )],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            request: rmcp::model::CallToolRequestParams,
            _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
            let title = request
                .arguments
                .as_ref()
                .and_then(|a| a.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(format!(
                    "http:{}:{}",
                    request.name,
                    title.to_lowercase()
                )),
            ]))
        }
    }

    /// Spin an axum + StreamableHttpService server on 127.0.0.1:0,
    /// mirroring rmcp's own test prior art. Returns the base URL.
    async fn spawn_http_test_server() -> (String, tokio::task::JoinHandle<()>) {
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };
        let service = StreamableHttpService::new(
            || Ok(HttpToolServer),
            std::sync::Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (format!("http://{addr}/mcp"), task)
    }

    /// Real HTTP round-trip over a socket: connect, list, register
    /// prefixed tools, dispatch under bare names.
    #[tokio::test]
    async fn connect_http_registers_prefixed_tools_over_socket() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::ToolContext;
        let (url, _server) = spawn_http_test_server().await;
        let cfg = ServerConfig {
            name: "web".to_string(),
            transport: Transport::StreamableHttp {
                url,
                headers: Default::default(),
                auth: false,
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let _conn = connect_http(&cfg, &handle, &state)
            .await
            .expect("http connect");
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["mcp__web__http_echo".to_string()]);
        let mut ctx = ToolContext::new();
        let result = handle
            .execute("mcp__web__http_echo", r#"{"text":"HeLLo"}"#, &mut ctx)
            .await;
        assert!(result.is_success(), "{result:?}");
        assert!(
            result.output().render().contains("http:http_echo:hello"),
            "got: {}",
            result.output().render()
        );
    }

    /// HTTP fake server that records headers it saw, for the
    /// static-headers test below.
    #[derive(Clone)]
    struct HeaderRecordingServer {
        seen_auth: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
    }

    impl rmcp::ServerHandler for HeaderRecordingServer {
        fn get_info(&self) -> rmcp::model::ServerInfo {
            rmcp::model::ServerInfo::new(
                rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
            )
            .with_protocol_version(rmcp::model::ProtocolVersion::LATEST)
            .with_server_info(rmcp::model::Implementation::new(
                "rem-mcp-hdr-test",
                "0.1.0",
            ))
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::ListToolsResult, rmcp::model::ErrorData> {
            use std::sync::Arc;
            Ok(rmcp::model::ListToolsResult {
                tools: vec![rmcp::model::Tool::new(
                    "ping",
                    "No-op probe",
                    Arc::new(serde_json::Map::new()),
                )],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            request: rmcp::model::CallToolRequestParams,
            context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
            // The tower service injects `http::request::Parts` into
            // request extensions (see handle_post in tower.rs).
            let auth = context
                .extensions
                .get::<http::request::Parts>()
                .and_then(|p| p.headers.get(http::header::AUTHORIZATION))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(missing)")
                .to_string();
            self.seen_auth.lock().await.push(auth);
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(format!("pong:{}", request.name)),
            ]))
        }
    }

    #[tokio::test]
    async fn connect_http_sends_static_headers() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::ToolContext;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };
        let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let server = HeaderRecordingServer {
            seen_auth: seen.clone(),
        };
        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            std::sync::Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer sekrit-test-token".to_string(),
        );
        let cfg = ServerConfig {
            name: "hdr".to_string(),
            transport: Transport::StreamableHttp {
                url: format!("http://{addr}/mcp"),
                headers,
                auth: false,
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let _conn = connect_http(&cfg, &handle, &state)
            .await
            .expect("http connect");
        let mut ctx = ToolContext::new();
        let result = handle.execute("mcp__hdr__ping", r#"{}"#, &mut ctx).await;
        assert!(result.is_success(), "{result:?}");
        let seen = seen.lock().await.clone();
        assert_eq!(
            seen,
            vec!["Bearer sekrit-test-token".to_string()],
            "got: {seen:?}"
        );
    }

    #[tokio::test]
    async fn connect_all_warns_and_continues_past_broken_http() {
        use rig::agent::tool::server::ToolServer;
        let (url, _server) = spawn_http_test_server().await;
        let loaded = LoadedConfig {
            servers: vec![
                ServerConfig {
                    name: "good".to_string(),
                    transport: Transport::StreamableHttp {
                        url,
                        headers: Default::default(),
                        auth: false,
                    },
                },
                ServerConfig {
                    name: "badhttp".to_string(),
                    // Nothing listens here: connect fails fast.
                    transport: Transport::StreamableHttp {
                        url: "http://127.0.0.1:1/mcp".to_string(),
                        headers: Default::default(),
                        auth: false,
                    },
                },
            ],
            warnings: vec![],
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let report = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            connect_all(&loaded, &handle, &state),
        )
        .await
        .expect("connect_all bounded");
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].contains("badhttp"),
            "got: {:?}",
            report.warnings
        );
        assert_eq!(report.inventory.len(), 1);
        assert_eq!(report.inventory[0].server, "good");
    }

    #[tokio::test]
    async fn mcp_toggle_off_removes_tools_and_skips_refresh() {
        use rig::agent::tool::server::ToolServer;
        use rmcp::ServiceExt;
        let tools = std::sync::Arc::new(tokio::sync::Mutex::new(vec!["alpha".to_string()]));
        let server = ChangingServer {
            tools: tools.clone(),
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let (c2s, sfc) = tokio::io::duplex(8192);
        let (s2c, cfs) = tokio::io::duplex(8192);
        // Capture the server-side peer while both ends are live, so the
        // test can notify over a live transport later.
        let (peer_tx, peer_rx) = tokio::sync::oneshot::channel();
        let server_task = tokio::spawn(async move {
            let running = server.serve((sfc, s2c)).await.expect("server start");
            let _ = peer_tx.send(running.peer().clone());
            let _ = running.waiting().await;
        });
        let client = RefreshHandler::new("one".to_string(), handle.clone(), state.clone())
            .serve((cfs, c2s))
            .await
            .expect("client connect");
        let server_peer = peer_rx.await.expect("server peer");
        // Adopt a record as connect_all would (dummy transport: unused
        // on the off path).
        state.lock().unwrap().push(McpServerRecord {
            name: "one".to_string(),
            status: McpConnStatus::Connected,
            enabled: true,
            transport: Transport::Stdio {
                command: "test-dummy".to_string(),
                args: vec![],
                env: Default::default(),
            },
            connection: Some(Connection { _service: client }),
        });
        // Sanity: while enabled, a live notify refreshes onto the handle.
        {
            let mut guard = tools.lock().await;
            *guard = vec!["beta".to_string()];
        }
        server_peer
            .notify_tool_list_changed()
            .await
            .expect("notify");
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let names: Vec<String> = handle
                    .get_tool_defs(None)
                    .await
                    .expect("defs")
                    .into_iter()
                    .map(|d| d.name)
                    .collect();
                if names.contains(&"mcp__one__beta".to_string()) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("refresh while enabled");
        // Toggle off: tools vanish from the handle.
        let msg = mcp_set_enabled(&state, &handle, "one", false)
            .await
            .expect("toggle off");
        assert!(msg.contains("off"), "got: {msg}");
        assert!(handle.get_tool_defs(None).await.expect("defs").is_empty());
        // A fresh connect while disabled must NOT re-register (serve
        // skip), and a live notify must NOT refresh back on either.
        // Toggle-off dropped the old transport, so build a new live pair.
        drop(server_peer);
        server_task.abort();
        let (c2s2, sfc2) = tokio::io::duplex(8192);
        let (s2c2, cfs2) = tokio::io::duplex(8192);
        let server2 = ChangingServer {
            tools: tools.clone(),
        };
        let (peer_tx2, peer_rx2) = tokio::sync::oneshot::channel();
        let server_task2 = tokio::spawn(async move {
            use rmcp::ServiceExt;
            let running = server2.serve((sfc2, s2c2)).await.expect("server start");
            let _ = peer_tx2.send(running.peer().clone());
            let _ = running.waiting().await;
        });
        let _disabled_client =
            RefreshHandler::new("one".to_string(), handle.clone(), state.clone())
                .serve((cfs2, c2s2))
                .await
                .expect("client connect");
        assert!(
            handle.get_tool_defs(None).await.expect("defs").is_empty(),
            "disabled serve registered tools"
        );
        {
            let mut guard = tools.lock().await;
            *guard = vec!["gamma".to_string()];
        }
        let server_peer2 = peer_rx2.await.expect("server peer 2");
        server_peer2
            .notify_tool_list_changed()
            .await
            .expect("notify 2");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            handle.get_tool_defs(None).await.expect("defs").is_empty(),
            "disabled server refreshed back on"
        );
        server_task2.abort();
    }

    #[tokio::test]
    async fn mcp_toggle_on_reconnects_and_reregisters() {
        use rig::agent::tool::server::ToolServer;
        let script = std::env::temp_dir().join("rem-mcp-py-toggle-on-test.py");
        std::fs::write(&script, PY_MCP_SERVER).unwrap();
        let cfg = ServerConfig {
            name: "py".to_string(),
            transport: Transport::Stdio {
                command: "python3".to_string(),
                args: vec![script.to_string_lossy().to_string()],
                env: Default::default(),
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        // Connect through the record-holding path, then toggle off.
        let loaded = LoadedConfig {
            servers: vec![cfg],
            warnings: vec![],
        };
        let report = connect_all(&loaded, &handle, &state).await;
        assert!(report.warnings.is_empty(), "got: {:?}", report.warnings);
        assert_eq!(handle.get_tool_defs(None).await.expect("defs").len(), 1);
        mcp_set_enabled(&state, &handle, "py", false)
            .await
            .expect("toggle off");
        assert!(handle.get_tool_defs(None).await.expect("defs").is_empty());
        // Toggle on reconnects through the recorded transport.
        let msg = mcp_set_enabled(&state, &handle, "py", true)
            .await
            .expect("toggle on");
        assert!(msg.contains("on"), "got: {msg}");
        let names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["mcp__py__echo_downcase".to_string()]);
        let _ = std::fs::remove_file(&script);
    }

    #[tokio::test]
    async fn mcp_toggle_unknown_server_errors() {
        use rig::agent::tool::server::ToolServer;
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let err = mcp_set_enabled(&state, &handle, "nope", false)
            .await
            .expect_err("unknown server");
        assert!(err.contains("nope"), "got: {err}");
    }

    #[test]
    fn token_store_roundtrips_per_server_at_0600() {
        let dir = std::env::temp_dir().join("rem-mcp-token-test-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp-tokens.json");
        let store = FileTokenStore::new(path.clone());
        // Empty store loads nothing.
        assert!(store.load_sync("github").is_none());
        // Save + reload round-trips the token.
        store
            .save_sync("github", "tok-123", &["read".to_string()])
            .expect("save");
        let loaded = store.load_sync("github").expect("reload");
        assert_eq!(loaded.access_token, "tok-123");
        // 0600: owner read/write only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "token file must be 0600, got {:o}",
                mode
            );
        }
        // Unknown server loads nothing; logout clears.
        assert!(store.load_sync("other").is_none());
        store.clear_sync("github");
        assert!(store.load_sync("github").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn auth_http_without_token_marks_needs_auth() {
        use rig::agent::tool::server::ToolServer;
        let (url, _server) = spawn_http_test_server().await;
        let cfg = ServerConfig {
            name: "oauth".to_string(),
            transport: Transport::StreamableHttp {
                url,
                headers: Default::default(),
                auth: true,
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        // No stored token: connect records needs-auth, registers
        // nothing, and warns with a login hint — session proceeds.
        let report = connect_all(
            &LoadedConfig {
                servers: vec![cfg],
                warnings: vec![],
            },
            &handle,
            &state,
        )
        .await;
        assert_eq!(report.inventory.len(), 0);
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].contains("oauth") && report.warnings[0].contains("login"),
            "got: {:?}",
            report.warnings
        );
        {
            let records = state.lock().unwrap();
            assert_eq!(records.len(), 1);
            assert!(matches!(records[0].status, McpConnStatus::NeedsAuth));
        }
        assert!(handle.get_tool_defs(None).await.expect("defs").is_empty());
    }

    #[tokio::test]
    async fn auth_http_with_stored_token_sends_bearer() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::ToolContext;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };
        let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let server = HeaderRecordingServer {
            seen_auth: seen.clone(),
        };
        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            std::sync::Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        // Store a token first (as `/mcp login` would).
        let dir = std::env::temp_dir().join("rem-mcp-auth-token-test-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileTokenStore::new(dir.join("mcp-tokens.json"));
        store
            .save_sync("hdr", "stored-bearer-abc", &[])
            .expect("save token");
        let cfg = ServerConfig {
            name: "hdr".to_string(),
            transport: Transport::StreamableHttp {
                url: format!("http://{addr}/mcp"),
                headers: Default::default(),
                auth: true,
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let _conn = connect_http_with_store(&cfg, &handle, &state, &store)
            .await
            .expect("http connect with token");
        let mut ctx = ToolContext::new();
        let result = handle.execute("mcp__hdr__ping", r#"{}"#, &mut ctx).await;
        assert!(result.is_success(), "{result:?}");
        let seen = seen.lock().await.clone();
        assert!(
            seen.iter().any(|h| h == "Bearer stored-bearer-abc"),
            "stored bearer never reached the server, got: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mcp_login_unknown_server_errors() {
        use rig::agent::tool::server::ToolServer;
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let dir = std::env::temp_dir().join("rem-mcp-login-test-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileTokenStore::new(dir.join("mcp-tokens.json"));
        let err = mcp_login(&state, &handle, "nope", &store)
            .await
            .expect_err("unknown server");
        assert!(err.contains("nope"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Auth-aware fake server for the 401 integration test: rejects any
    /// bearer except the current good token with a 401-style tool error
    /// (MCP has no status codes on the tool path; the message is the
    /// contract), records every bearer it saw.
    #[derive(Clone)]
    struct RotatingTokenServer {
        good_token: std::sync::Arc<tokio::sync::Mutex<String>>,
        calls: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
    }

    impl rmcp::ServerHandler for RotatingTokenServer {
        fn get_info(&self) -> rmcp::model::ServerInfo {
            rmcp::model::ServerInfo::new(
                rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
            )
            .with_protocol_version(rmcp::model::ProtocolVersion::LATEST)
            .with_server_info(rmcp::model::Implementation::new(
                "rem-mcp-401-test",
                "0.1.0",
            ))
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::ListToolsResult, rmcp::model::ErrorData> {
            use std::sync::Arc;
            Ok(rmcp::model::ListToolsResult {
                tools: vec![rmcp::model::Tool::new(
                    "ping",
                    "No-op probe",
                    Arc::new(serde_json::Map::new()),
                )],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            request: rmcp::model::CallToolRequestParams,
            context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
            let bearer = context
                .extensions
                .get::<http::request::Parts>()
                .and_then(|p| p.headers.get(http::header::AUTHORIZATION))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(missing)")
                .to_string();
            self.calls.lock().await.push(bearer.clone());
            let good = format!("Bearer {}", self.good_token.lock().await.clone());
            if bearer != good {
                // 401-style tool error (MCP has no status codes on the
                // tool path; the message is the contract).
                return Ok(rmcp::model::CallToolResult::error(vec![
                    rmcp::model::ContentBlock::text("401 Unauthorized: invalid_token"),
                ]));
            }
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(format!("pong:{}", request.name)),
            ]))
        }
    }

    /// Mid-session 401: the dispatch callback sees an auth-shaped
    /// failure, marks the server needs-auth, and returns a
    /// model-visible error naming the server (replan hint, not hang).
    #[tokio::test]
    async fn auth_dispatch_failure_marks_needs_auth_with_server_named() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::{ToolContext, ToolErrorKind};
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        // Adopt a connected record with a (dummy) HTTP transport.
        state.lock().unwrap().push(McpServerRecord {
            name: "web".to_string(),
            status: McpConnStatus::Connected,
            enabled: true,
            transport: Transport::StreamableHttp {
                url: "http://127.0.0.1:1/mcp".to_string(),
                headers: Default::default(),
                auth: true,
            },
            connection: None,
        });
        // Simulate the 401-shaped dispatch failure path directly:
        // the helper classifies + marks + renders the model error.
        let err = auth_failure_needs_auth(&state, "web", "401 Unauthorized: invalid_token")
            .expect_err("must mark needs-auth");
        assert!(err.contains("web"), "names the server, got: {err}");
        assert!(err.contains("/mcp login web"), "login hint, got: {err}");
        {
            let records = state.lock().unwrap();
            assert!(matches!(records[0].status, McpConnStatus::NeedsAuth));
        }
        // Tools retire so the model can't keep calling a dead server.
        handle
            .add_dynamic_tool(rig::tool::DynamicTool::new(
                "mcp__web__ping",
                "probe",
                serde_json::json!({"type": "object"}),
                |_ctx, _args| Box::pin(async move { Ok(rig::tool::ToolOutput::text("x")) }),
            ))
            .await;
        retire_server_tools(&handle, "web").await;
        let result = handle
            .execute("mcp__web__ping", r#"{}"#, &mut ToolContext::new())
            .await;
        assert!(
            result.is_error_kind(ToolErrorKind::NotFound),
            "retired tool must fail replannable, got: {result:?}"
        );
    }

    /// Live 401 over a socket: stale bearer → 401 tool error →
    /// refresh attempt (no refresh token → fails) → needs-auth +
    /// server-naming model error + tools retired. Uses the real
    /// dispatch path through the shared handle.
    #[tokio::test]
    async fn live_401_marks_needs_auth_with_server_named() {
        use rig::agent::tool::server::ToolServer;
        use rig::tool::ToolContext;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };
        let good_token = std::sync::Arc::new(tokio::sync::Mutex::new("rotated-ok".to_string()));
        let calls = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let server = RotatingTokenServer {
            good_token: good_token.clone(),
            calls: calls.clone(),
        };
        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            std::sync::Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        // Token store with a STALE token and no refresh token: refresh
        // must fail → needs-auth.
        let dir = std::env::temp_dir().join("rem-mcp-401-live-test-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileTokenStore::new(dir.join("mcp-tokens.json"));
        store.save_sync("live401", "old-stale", &[]).expect("save");
        let cfg = ServerConfig {
            name: "live401".to_string(),
            transport: Transport::StreamableHttp {
                url: format!("http://{addr}/mcp"),
                headers: Default::default(),
                auth: true,
            },
        };
        let handle = ToolServer::new().run();
        let state: SharedMcpState = Default::default();
        let conn = connect_http_with_store(&cfg, &handle, &state, &store)
            .await
            .expect("connect with stale token");
        // Adopt like production `connect_all` does: the 401 path needs
        // the record for refresh URL lookup + needs-auth marking.
        adopt_connected(&state, &cfg, conn);
        // One dispatch over the wire: stale bearer → 401 → refresh
        // (fails, no refresh token) → needs-auth + retired tools.
        let mut ctx = ToolContext::new();
        let result = handle
            .execute("mcp__live401__ping", r#"{}"#, &mut ctx)
            .await;
        assert!(result.is_error(), "401 must surface, got: {result:?}");
        let rendered = result.output().render();
        // Model-visible error names the server with a login hint.
        let model_text = format!("{rendered} {:?}", result.error().map(|e| e.to_string()));
        assert!(
            model_text.contains("live401"),
            "names server, got: {model_text}"
        );
        assert!(
            model_text.contains("/mcp login"),
            "login hint, got: {model_text}"
        );
        // Record marked needs-auth, tools retired.
        {
            let records = state.lock().unwrap();
            let rec = records
                .iter()
                .find(|r| r.name == "live401")
                .expect("record");
            assert!(
                matches!(rec.status, McpConnStatus::NeedsAuth),
                "got: {:?}",
                rec.status
            );
        }
        assert!(
            handle.get_tool_defs(None).await.expect("defs").is_empty(),
            "tools must retire on 401"
        );
        // The stale bearer actually rode the wire (proves dispatch, not stub).
        let calls = calls.lock().await.clone();
        assert!(
            calls.iter().any(|c| c == "Bearer old-stale"),
            "stale bearer never hit the server, got: {calls:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn needs_auth_login_hints_name_the_server() {
        // Pure shape of the startup auto-login hint (no I/O): each
        // needs-auth record maps to one printable login line.
        let hints = needs_auth_login_hints(&["github".to_string(), "linear".to_string()]);
        assert_eq!(hints.len(), 2);
        assert!(
            hints[0].contains("github") && hints[0].contains("/mcp login github"),
            "got: {}",
            hints[0]
        );
        assert!(hints[1].contains("linear"), "got: {}", hints[1]);
        assert!(needs_auth_login_hints(&[]).is_empty());
    }

    #[test]
    fn sanitize_server_name_keeps_claude_safe_chars() {
        assert_eq!(sanitize_server_name("github"), "github");
        assert_eq!(sanitize_server_name("my-server_2"), "my-server_2");
        assert_eq!(sanitize_server_name("a b/c.d:e"), "a_b_c_d_e");
    }

    #[test]
    fn default_path_is_mcp_json_next_to_config() {
        let path = default_path().expect("config dir");
        assert!(path.ends_with("rem/mcp.json"), "got: {}", path.display());
        assert_eq!(
            path.parent().map(|p| p.to_path_buf()),
            crate::config::Config::path()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        );
    }

    #[test]
    fn missing_file_means_zero_servers_no_error() {
        let dir = std::env::temp_dir().join("rem-mcp-test-missing-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let loaded = load_from_path(&dir.join("mcp.json"));
        assert!(loaded.servers.is_empty());
        assert!(loaded.warnings.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_entry_warns_and_skips() {
        let json = serde_json::json!({
            "mcpServers": {
                "good": {"command": "uvx", "args": ["x"]},
                "bad": {"bogus": true}
            }
        });
        let loaded = parse_config(&json);
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers[0].name, "good");
        assert_eq!(loaded.warnings.len(), 1);
        assert!(
            loaded.warnings[0].contains("bad"),
            "got: {}",
            loaded.warnings[0]
        );
    }

    #[test]
    fn url_selects_http_command_selects_stdio() {
        let json = serde_json::json!({
            "mcpServers": {
                "remote": {"url": "https://mcp.example.com/mcp"},
                "local": {"command": "uvx"}
            }
        });
        let loaded = parse_config(&json);
        assert_eq!(loaded.servers.len(), 2);
        assert!(matches!(
            loaded
                .servers
                .iter()
                .find(|s| s.name == "remote")
                .unwrap()
                .transport,
            Transport::StreamableHttp { .. }
        ));
        assert!(matches!(
            loaded
                .servers
                .iter()
                .find(|s| s.name == "local")
                .unwrap()
                .transport,
            Transport::Stdio { .. }
        ));
    }

    #[test]
    fn env_vars_expand_with_defaults() {
        // SAFETY: single-threaded test mutating a test-only var.
        unsafe {
            std::env::set_var("REM_MCP_TEST_TOKEN", "sekrit");
            std::env::remove_var("REM_MCP_TEST_UNSET_XYZ");
        }
        assert_eq!(expand_env("Bearer ${REM_MCP_TEST_TOKEN}"), "Bearer sekrit");
        assert_eq!(
            expand_env("${REM_MCP_TEST_UNSET_XYZ:-fallback}"),
            "fallback"
        );
        assert_eq!(expand_env("${REM_MCP_TEST_UNSET_XYZ}"), "");
        // SAFETY: restoring test-only var.
        unsafe {
            std::env::remove_var("REM_MCP_TEST_TOKEN");
        }
    }
}
