use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rig::{
    agent::AgentBuilder,
    agent::{
        AgentHook, CompletionResponseEvent, HookContext, ObservationAction, ToolResultAction,
        ToolResultEvent,
    },
    client::CompletionClient,
    completion::{Chat, Prompt},
    message::{AssistantContent, ToolChoice},
    providers::{anthropic, openai},
};

use crate::{
    config::{Config, ProviderKind},
    context::Context,
    modes::Mode,
    permissions::{ApprovalTx, PermissionHook},
    tools::{
        BashTool, EditTool, GitDiffTool, GitStatusTool, GlobTool, GrepTool, ListDirectoryTool,
        ReadTool, WebFetchTool, WebSearchTool, WriteTool,
    },
};

/// Swappable orchestrator abstraction. `RigAgent` is the Rig implementation;
/// a different loop (hand-rolled ReAct, another framework) implements this trait.
#[async_trait]
pub trait AgentLoop {
    async fn chat(&self, prompt: &str) -> Result<String, String>;
    /// Tool calls recorded during the most recent `chat` turn, in order.
    /// Empty before the first turn. Used by the TUI to render the Aster-style
    /// work tree. Defaulted so other `AgentLoop` impls need no changes.
    fn last_tool_events(&self) -> Vec<ToolEvent> {
        Vec::new()
    }
    /// True LLM reasoning texts captured during the most recent `chat` turn,
    /// in order. Empty when the provider emitted no `Reasoning` blocks.
    /// Defaulted so other `AgentLoop` impls need no changes. Recorded
    /// internally but hidden from the transcript (ADR-0005).
    #[allow(dead_code)]
    fn last_reasoning(&self) -> Vec<String> {
        Vec::new()
    }
    /// Model id for the TUI footer. Defaulted for other impls.
    fn model_name(&self) -> String {
        "model".to_string()
    }
    /// Effort level for the TUI footer and the `reasoning_effort` request
    /// param. Defaulted for other impls.
    fn effort_name(&self) -> String {
        "medium".to_string()
    }
    fn mode_handle(&self) -> Arc<Mutex<Mode>> {
        Arc::new(Mutex::new(Mode::default()))
    }
    /// Discovered skills for `/skills` list + `/skill:<name>` invoke.
    /// Defaulted so other `AgentLoop` impls report none.
    fn list_skills(&self) -> Vec<SkillSummary> {
        Vec::new()
    }
    /// Full `SKILL.md` body for one skill by folder name. Default: none.
    fn get_skill_body(&self, _name: &str) -> Option<String> {
        None
    }
    /// Re-scan skill dirs + rebuild preamble for subsequent turns.
    /// Returns human summary (`reloaded N skills: a, b`). Default: unsupported.
    fn reload_skills_sync(&self) -> Result<String, String> {
        Err("not supported".to_string())
    }
    /// Switch the active provider/model mid-Session. `spec` is `model`
    /// (same provider) or `provider:model` (cross provider). Preserves
    /// caller-owned history; returns the new `provider:model` tag.
    /// Default: unsupported. Caller (TUI) must refuse while busy.
    fn switch_model_sync(&self, _spec: &str) -> Result<String, String> {
        Err("not supported".to_string())
    }
    /// Set the session-local reasoning effort (`low`/`medium`/`high`).
    /// Returns the normalized level. Default: unsupported.
    fn set_effort_sync(&self, _level: &str) -> Result<String, String> {
        Err("not supported".to_string())
    }
    /// All configured `provider:model` combos for `/model` listing.
    /// Default: none.
    fn list_models(&self) -> Vec<String> {
        Vec::new()
    }
    /// Local usage snapshot for `/context`: message count, char estimate,
    /// compaction trigger + keep window, summary presence. No token API.
    /// Default: zeros. Lock failures report zeros, never block.
    fn context_stats_sync(&self) -> crate::context::ContextStats {
        crate::context::ContextStats::default()
    }
    /// Connected MCP servers for the `/mcp` panel. Default: none.
    /// `RigAgent` reports live status + counts + session enable flags.
    fn list_mcp_servers(&self) -> Vec<McpServerSummary> {
        Vec::new()
    }
    /// Enable/disable one MCP server for the session scope. Takes effect
    /// immediately (tools unregister/re-register on the shared handle).
    /// Default: unsupported (alternate impls without MCP).
    fn set_mcp_server_enabled(&self, _name: &str, _enabled: bool) -> Result<String, String> {
        Err("not supported".to_string())
    }
    /// Begin OAuth login for one MCP server. Returns the authorization
    /// URL to open (caller prints it + tries the system browser).
    /// Default: unsupported (alternate impls without MCP).
    fn mcp_login(&self, _name: &str) -> Result<String, String> {
        Err("not supported".to_string())
    }
    /// Log out of one MCP server: clear the stored token, drop the
    /// connection, mark needs-auth. Default: unsupported.
    fn mcp_logout(&self, _name: &str) -> Result<String, String> {
        Err("not supported".to_string())
    }
    // Trait surface for alternate `AgentLoop` impls; `RigAgent` overrides it.
    #[allow(dead_code)]
    async fn export_messages_json(&self) -> Result<String, String> {
        Err("not supported".to_string())
    }
    async fn import_messages_json(&self, _json: &str) -> Result<(), String> {
        Err("not supported".to_string())
    }
    fn export_sync(&self) -> Result<String, String> {
        Err("not supported".to_string())
    }
    fn import_sync(&self, _json: &str) -> Result<(), String> {
        Err("not supported".to_string())
    }
}

/// One MCP server for the `/mcp` panel: identity + connection status +
/// tool count + session enable flag. No handles across the trait — the
/// TUI renders text; toggling goes through trait methods.
#[derive(Debug, Clone)]
pub struct McpServerSummary {
    pub name: String,
    pub status: McpServerStatus,
    pub tool_count: usize,
    pub enabled: bool,
}

/// Connection status for one MCP server in the `/mcp` panel.
#[derive(Debug, Clone)]
pub enum McpServerStatus {
    /// Spawned/connected and tools registered.
    Connected,
    /// Spawn/connect/list failed; carries the warn-and-continue reason.
    Failed { reason: String },
    /// HTTP 401 path (Ticket 06): needs `/mcp login <name>`.
    NeedsAuth,
}

/// One discovered skill for the `/skills` list: identity + listing fields only.
/// No `PathBuf` across the trait — TUI renders `path` as text; bodies load
/// via [`AgentLoop::get_skill_body`].
#[derive(Debug, Clone)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub path: String,
    pub note: Option<String>,
}

/// One tool execution observed during a turn: what ran, with what args,
/// whether it succeeded, a short human-readable result summary, and the full
/// output text (for inline expandable display).
#[derive(Debug, Clone)]
pub struct ToolEvent {
    pub name: String,
    pub args: serde_json::Value,
    pub ok: bool,
    pub summary: String,
    /// Full rendered tool output; may be multi-line. Truncated to
    /// [`FULL_OUTPUT_CAP`] chars so the transcript stores a bounded copy.
    pub output: String,
}

impl ToolEvent {
    /// Short arg preview for the tree leaf, e.g. `src/main.rs` or `cargo test`.
    pub fn arg_preview(&self) -> String {
        preview_args(&self.name, &self.args)
    }
}

fn preview_args(name: &str, args: &serde_json::Value) -> String {
    let get = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_string);
    match name {
        "read" | "write" | "edit" | "list_directory" => {
            get("path").unwrap_or_else(|| compact_args(args))
        }
        "bash" => get("command").unwrap_or_else(|| compact_args(args)),
        "grep" => match (get("pattern"), get("path")) {
            (Some(p), Some(path)) => format!("{p} in {path}"),
            (Some(p), None) => p,
            _ => compact_args(args),
        },
        "glob" => get("pattern").unwrap_or_else(|| compact_args(args)),
        "git_diff" => get("path").unwrap_or_else(|| "repo".to_string()),
        "git_status" => "repo".to_string(),
        "web_fetch" => get("url").unwrap_or_else(|| compact_args(args)),
        "web_search" => get("query").unwrap_or_else(|| compact_args(args)),
        _ => compact_args(args),
    }
}

fn compact_args(args: &serde_json::Value) -> String {
    const N: usize = 80;
    let s = args.to_string();
    match s.chars().count() > N {
        true => format!("{}…", s.chars().take(N - 1).collect::<String>()),
        false => s,
    }
}

/// Bound on the full output text stored per tool event for inline display.
/// Long outputs render truncated with expand; the stored copy stays bounded.
pub const FULL_OUTPUT_CAP: usize = 4000;

/// [`AgentHook`] that records every tool result plus true LLM reasoning text
/// into shared vecs. Registered via `AgentBuilder::add_hook`; fires for
/// successes, failures, skips, and refusals without altering agent behavior
/// (always `Keep` / `Continue`).
#[derive(Debug, Clone, Default)]
struct ToolRecorder {
    events: Arc<Mutex<Vec<ToolEvent>>>,
    reasoning: Arc<Mutex<Vec<String>>>,
}

impl AgentHook for ToolRecorder {
    /// Capture true LLM reasoning text from each model response. Only
    /// `Reasoning` content blocks count; plain text and tool calls are not
    /// reasoning. Never alters behavior (always `Continue`).
    async fn on_completion_response(
        &self,
        _ctx: &HookContext,
        event: CompletionResponseEvent<'_>,
    ) -> ObservationAction {
        let mut texts = Vec::new();
        for content in event.content.iter() {
            if let AssistantContent::Reasoning(r) = content {
                let text = r.display_text();
                if !text.trim().is_empty() {
                    texts.push(text);
                }
            }
        }
        if !texts.is_empty()
            && let Ok(mut guard) = self.reasoning.lock()
        {
            guard.extend(texts);
        }
        ObservationAction::Continue
    }

    async fn on_tool_result(
        &self,
        _ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let ok = event.raw_result.is_success() || event.raw_result.is_skipped();
        let detail = event.presentation.render();
        let summary = match ok {
            true => first_line(&detail, 120),
            false => format!("failed: {}", first_line(&detail, 120)),
        };
        let output = truncate_chars(&detail, FULL_OUTPUT_CAP);
        let parsed: serde_json::Value =
            serde_json::from_str(event.args).unwrap_or(serde_json::Value::Null);
        if let Ok(mut guard) = self.events.lock() {
            guard.push(ToolEvent {
                name: event.tool_name.to_string(),
                args: parsed,
                ok,
                summary,
                output,
            });
        }
        ToolResultAction::keep()
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.chars().count() > max {
        true => format!(
            "{}… [truncated]",
            s.chars().take(max - 1).collect::<String>()
        ),
        false => s.to_string(),
    }
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    const ELLIPSIS: char = '…';
    match line.chars().count() > max {
        true => format!(
            "{}…",
            line.chars().take(max - 1).collect::<String>().trim_end()
        ),
        false => line.to_string().replace(ELLIPSIS, "..."),
    }
}

/// Rig-backed agent: OpenAI-compatible `/chat/completions` model + preamble +
/// the 11 file/shell/git/search/web tools, using Rig's built-in multi-step loop (no hand-rolled ReAct).
/// Conversation history is caller-owned in [`Context`]: tool outputs are
/// truncated on entry and old turns are summarized past 100 messages.
pub struct RigAgent {
    inner: tokio::sync::RwLock<rig::agent::Agent>,
    context: tokio::sync::Mutex<Context>,
    recorder: ToolRecorder,
    /// Active provider + model + effort, mutated live by `/model` and
    /// `/effort`. Interior-mutable (same `Arc<Mutex>` shape as `mode`)
    /// so the sync TUI path can switch through `&self`.
    switch_state: Arc<Mutex<SwitchState>>,
    mode: Arc<Mutex<Mode>>,
    // Rebuild inputs for `/skills reload`: the inner `Agent` is frozen at
    // build time, so reload re-runs `build_inner` with a fresh skill scan.
    cfg: Config,
    project_root: std::path::PathBuf,
    gate: PermissionHook,
    skills: Arc<Mutex<Vec<crate::skills::Skill>>>,
    // Shared tool registry: 11 built-ins + runtime-added MCP tools.
    // Held here (not rebuilt) so `/skills reload` and preamble rebuilds
    // never drop MCP registrations.
    tool_handle: rig::agent::tool::server::ToolServerHandle,
    // Rendered MCP preamble index, remembered so `/skills reload`
    // preserves it. Ticket 05 recomputes it on `/mcp` toggles.
    // Interior mutability: main connects MCP after construction.
    mcp_section: std::sync::Mutex<String>,
    // Live MCP records: one per configured server. Shared with each
    // `RefreshHandler` so `list_changed` refresh skips disabled
    // servers; connections live here for the process lifetime.
    mcp_state: crate::mcp::SharedMcpState,
}

/// Build the shared tool registry: all 11 static built-ins on one
/// `ToolServerHandle` so later MCP tools can join the same registry.
fn build_tool_server() -> rig::agent::tool::server::ToolServerHandle {
    use rig::agent::tool::server::ToolServer;
    ToolServer::new()
        .tool(ReadTool)
        .tool(WriteTool)
        .tool(EditTool)
        .tool(BashTool)
        .tool(ListDirectoryTool)
        .tool(GitStatusTool)
        .tool(GitDiffTool)
        .tool(GrepTool)
        .tool(GlobTool)
        .tool(WebFetchTool)
        .tool(WebSearchTool)
        .run()
}

/// Live provider/model/effort selection for one `RigAgent`.
/// Session-local: `/model` mutates provider + model, `/effort` mutates
/// effort. The stored `Config` keeps canonical entries; this keeps the
/// live pointer.
#[derive(Debug, Clone)]
struct SwitchState {
    provider: String,
    model: String,
    effort: String,
}

impl SwitchState {
    fn tag(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }
}

/// Project instructions file name, loaded from the project root only.
const AGENTS_MD: &str = "AGENTS.md";
/// Header separating static instructions from project context in the preamble.
const AGENTS_MD_HEADER: &str = "\n\nProject context from AGENTS.md:\n";

/// Normalize a user-facing effort level to `low` / `medium` / `high`.
/// Unset or unrecognized values fall back to `medium`.
pub fn normalize_effort(raw: &str) -> &str {
    match raw {
        "low" | "high" => raw,
        _ => "medium",
    }
}

/// Unified effort → Anthropic thinking budget + required max tokens.
/// Returns `(budget_tokens, max_tokens)`; max always exceeds budget.
pub fn anthropic_budget(effort: &str) -> (u64, u64) {
    match effort {
        "low" => (1024, 4096),
        "high" => (8192, 16384),
        _ => (4096, 8192),
    }
}

/// Fixed retry budget for transient provider failures (ticket 03).
const MAX_RETRY_ATTEMPTS: u32 = 3;
/// Backoff schedule: exponential from a 500ms base, `2^attempt * base`.
fn retry_delay(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(500 * 2u64.pow(attempt))
}

/// Run `op` with fixed retry on transient failures: up to
/// [`MAX_RETRY_ATTEMPTS`] attempts with [`retry_delay`] backoff between
/// attempts 1..N. Non-retryable errors return immediately. The sleep is
/// the real contract (tests pay ~1.5s on the retry paths).
async fn with_retry<T, F, Fut>(mut op: F) -> Result<T, String>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut attempt = 0;
    loop {
        match op(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) if !is_retryable_error(&e) => return Err(e),
            Err(e) if attempt + 1 >= MAX_RETRY_ATTEMPTS => return Err(e),
            Err(_) => {
                tokio::time::sleep(retry_delay(attempt)).await;
                attempt += 1;
            }
        }
    }
}

/// True when a stringified provider error looks transient: rate limits,
/// 5xx server failures, timeouts, and dropped connections. Everything
/// else (auth, bad request, unknown model) fails fast without retry.
fn is_retryable_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("internal server error")
        || lower.contains("service unavailable")
        || lower.contains("bad gateway")
        || lower.contains("gateway timeout")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("connection aborted")
        || lower.contains("temporarily unavailable")
}

/// Stage a config pointed at one provider/model target: clone the stored
/// config, move the active pointer, and override that entry's default
/// model. Shared by startup, `/model`, `/effort`, and `/skills reload`
/// so key/endpoint/params always resolve identically.
fn staged_config(
    cfg: &Config,
    provider: &str,
    model: &str,
    effort: &str,
) -> Result<Config, String> {
    let mut staged = cfg.clone();
    staged.active_provider = provider.to_string();
    staged.effort = effort.to_string();
    staged
        .providers
        .get_mut(provider)
        .map(|entry| entry.default_model = model.to_string())
        .ok_or_else(|| format!("unknown provider \"{provider}\""))?;
    Ok(staged)
}

/// Load project-root `AGENTS.md` verbatim. No cap, no transform.
/// Returns `None` when missing/unreadable (caller silently skips).
fn load_agents_md(project_root: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(project_root.join(AGENTS_MD)).ok()
}

/// Compose the agent preamble: static instructions + project context +
/// skills index + MCP index. Pure constructor — called at startup and on
/// `/skills reload`. `mcp_section` is pre-rendered by the MCP module
/// (empty when no servers); Ticket 05 refreshes it on `/mcp` toggles.
fn build_preamble(
    project_root: &std::path::Path,
    skills: &[crate::skills::Skill],
    mcp_section: &str,
) -> String {
    let mut preamble = String::from(
        "You are rem, a coding agent in a terminal TUI. \
         Use the read/write/edit/bash/list_directory/git_status/git_diff/grep/glob/web_fetch/web_search tools to inspect and change files. \
         list_directory lists a dir, glob finds files by pattern, grep searches contents, git_status/git_diff inspect git state. \
         web_search searches the web (DuckDuckGo, no key), web_fetch reads a URL as text. \
         Bash runs `sh -c` in the project dir (30s timeout). Destructive shell patterns are blocked outright; other mutations and network access ask the human for approval mid-run — if a call is denied, replan without it. \
         Prefer reading a file before editing it. Keep replies concise.",
    );
    if let Some(body) = load_agents_md(project_root) {
        preamble.push_str(AGENTS_MD_HEADER);
        preamble.push_str(&body);
    }
    if !skills.is_empty() {
        preamble.push_str(&crate::skills::preamble_section(skills));
    }
    preamble.push_str(mcp_section);
    preamble
}

impl RigAgent {
    pub fn new(
        cfg: &Config,
        approval_tx: ApprovalTx,
        project_root: std::path::PathBuf,
    ) -> Result<Self, String> {
        Self::new_with_mcp_section(cfg, approval_tx, project_root, "")
    }

    /// Constructor with a pre-rendered MCP preamble index. The caller
    /// (`main`) connects MCP servers first, then passes the section so
    /// the frozen preamble advertises connected tools from turn one.
    /// `/skills reload` rebuilds with the same section; Ticket 05
    /// refreshes it on `/mcp` toggles.
    pub fn new_with_mcp_section(
        cfg: &Config,
        approval_tx: ApprovalTx,
        project_root: std::path::PathBuf,
        mcp_section: &str,
    ) -> Result<Self, String> {
        let recorder = ToolRecorder::default();
        let mode = Arc::new(Mutex::new(Mode::default()));
        // Observer first (see everything), gate second (steers). ADR-0001.
        let gate = PermissionHook::new(approval_tx, project_root.clone(), mode.clone());
        let skills = crate::skills::discover(&project_root);
        let tool_handle = build_tool_server();
        let mcp_state: crate::mcp::SharedMcpState = Default::default();

        let resolved = cfg.active()?;
        let effort = normalize_effort(&cfg.effort).to_string();
        // Resolve + validate the startup target before building so a bad
        // key/endpoint fails here, not inside the builder.
        let staged = staged_config(cfg, &resolved.name, &resolved.model, &effort)?;
        let agent = Self::build_inner(
            &staged,
            &effort,
            &gate,
            &recorder,
            &project_root,
            &skills,
            &tool_handle,
            mcp_section,
        )?;

        Ok(Self {
            inner: tokio::sync::RwLock::new(agent),
            context: tokio::sync::Mutex::new(Context::new()),
            recorder,
            switch_state: Arc::new(Mutex::new(SwitchState {
                provider: resolved.name,
                model: resolved.model,
                effort,
            })),
            mode,
            cfg: cfg.clone(),
            project_root,
            gate,
            skills: Arc::new(Mutex::new(skills)),
            tool_handle,
            mcp_section: std::sync::Mutex::new(mcp_section.to_string()),
            mcp_state,
        })
    }

    /// Shared live MCP records for `/mcp` + refresh handlers.
    pub fn mcp_state(&self) -> &crate::mcp::SharedMcpState {
        &self.mcp_state
    }

    /// The shared tool registry (built-ins + MCP tools). Ticket 03's
    /// `connect_all` registers onto this; preamble rebuilds reuse it.
    pub fn tool_server_handle(&self) -> rig::agent::tool::server::ToolServerHandle {
        self.tool_handle.clone()
    }

    /// Build the inner `rig::agent::Agent`: client + preamble (static +
    /// AGENTS.md + skills index) + 11 tools + hooks. Pure constructor —
    /// called at startup, on `/skills reload`, and on `/model` switches.
    /// Branches on the active provider entry: OpenAI-compatible legs keep
    /// chat-completions semantics, Anthropic legs use the native Messages
    /// client with a unified-effort thinking budget.
    /// `effective` is the staged config (active pointer + default model
    /// already pointed at the target); `effort` is the session-local level.
    #[allow(clippy::too_many_arguments)]
    fn build_inner(
        effective: &Config,
        effort: &str,
        gate: &PermissionHook,
        recorder: &ToolRecorder,
        project_root: &std::path::Path,
        skills: &[crate::skills::Skill],
        tool_handle: &rig::agent::tool::server::ToolServerHandle,
        mcp_section: &str,
    ) -> Result<rig::agent::Agent, String> {
        let resolved = effective.active()?;
        let effort = normalize_effort(effort);

        // Type-erased model handle: `Agent` carries no provider type
        // parameter, so switching providers is just a rebuild.
        let handle: rig::agent::ModelHandle = match resolved.kind {
            ProviderKind::OpenAICompatible => {
                let base_url = resolved.base_url.clone().ok_or_else(|| {
                    format!(
                        "provider \"{}\" type openai-compatible requires base_url",
                        resolved.name
                    )
                })?;
                // Explicit `.base_url()` per spec — no reliance on OPENAI_BASE_URL env.
                let client = openai::CompletionsClient::builder()
                    .api_key(resolved.key.clone())
                    .base_url(base_url)
                    .build()
                    .map_err(|e| format!("failed to build LLM client: {e}"))?;
                rig::agent::ModelHandle::new(client.completion_model(resolved.model.clone()))
            }
            ProviderKind::Anthropic => {
                let base_url = resolved
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com".to_string());
                let client = anthropic::Client::builder()
                    .api_key(resolved.key.clone())
                    .base_url(base_url)
                    .build()
                    .map_err(|e| format!("failed to build LLM client: {e}"))?;
                rig::agent::ModelHandle::new(client.completion_model(resolved.model.clone()))
            }
        };

        let additional_params = match resolved.kind {
            ProviderKind::OpenAICompatible => {
                serde_json::json!({"reasoning_effort": effort})
            }
            ProviderKind::Anthropic => {
                let (budget, max_tokens) = anthropic_budget(effort);
                serde_json::json!({
                    "thinking": {"type": "enabled", "budget_tokens": budget},
                    "max_tokens": max_tokens,
                })
            }
        };

        let preamble = build_preamble(project_root, skills, mcp_section);

        let agent = AgentBuilder::from_model_handle(handle)
            .preamble(&preamble)
            .tool_server_handle(tool_handle.clone())
            .default_max_turns(100)
            .additional_params(additional_params)
            .add_hook(recorder.clone())
            .add_hook(gate.clone())
            .build();

        Ok(agent)
    }

    /// Provider-qualified tag for the footer and Session record
    /// (`provider:model`).
    pub fn provider_tag(&self) -> String {
        self.switch_state
            .lock()
            .map(|s| s.tag())
            .unwrap_or_else(|_| "model".to_string())
    }

    // Wired up by upcoming session-title work (`generate_title` has a TODO
    // at the call site); kept warning-free until then.
    #[allow(dead_code)]
    pub async fn message_count(&self) -> usize {
        self.context.lock().await.len()
    }

    #[allow(dead_code)]
    pub async fn generate_title(&self) -> Result<String, String> {
        self.inner
            .read()
            .await
            .prompt("Give this coding session a short 2-5 word title, reply with title only")
            .preamble("You name coding sessions. Reply with a short title only.")
            .tool_choice(ToolChoice::None)
            .await
            .map_err(|e| e.to_string())
            .map(|s| s.trim().to_string())
    }

    /// Rebuild the preamble index from the current enabled inventory.
    /// Shared by toggle + logout + login-callback paths. Same shared
    /// handle, so tools survive. Fail-closed on rebuild error.
    fn rebuild_mcp_preamble(&self) -> Result<(), String> {
        let section = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                crate::mcp::preamble_section(
                    &crate::mcp::inventory_for_enabled(&self.mcp_state, &self.tool_handle).await,
                )
            })
        });
        let fresh = crate::skills::discover(&self.project_root);
        // Stage through the live switch state so an MCP toggle never
        // reverts a `/model` or `/effort` switch (union of both branches).
        let (staged, effort) = {
            let state = self
                .switch_state
                .lock()
                .map_err(|_| "agent busy, try again".to_string())?;
            let staged =
                staged_config(&self.cfg, &state.provider, &state.model, &state.effort)?;
            (staged, state.effort.clone())
        };
        let rebuilt = Self::build_inner(
            &staged,
            &effort,
            &self.gate,
            &self.recorder,
            &self.project_root,
            &fresh,
            &self.tool_handle,
            &section,
        )?;
        match self.inner.try_write() {
            Ok(mut guard) => {
                *guard = rebuilt;
                if let Ok(mut s) = self.mcp_section.lock() {
                    *s = section;
                }
            }
            Err(_) => return Err("agent busy, try again".to_string()),
        }
        Ok(())
    }

    /// Rebuild the preamble with a fresh MCP index after startup
    /// connects servers. Same shared handle, so tools survive.
    /// Fail-closed: rebuild error keeps the old agent.
    pub fn rebuild_preamble_with_mcp_section(&mut self, mcp_section: &str) -> Result<(), String> {
        let fresh = self.skills.lock().map(|g| g.clone()).unwrap_or_default();
        // Stage through the live switch state (see rebuild_mcp_preamble).
        let (staged, effort) = {
            let state = self
                .switch_state
                .lock()
                .map_err(|_| "agent busy, try again".to_string())?;
            let staged =
                staged_config(&self.cfg, &state.provider, &state.model, &state.effort)?;
            (staged, state.effort.clone())
        };
        let rebuilt = Self::build_inner(
            &staged,
            &effort,
            &self.gate,
            &self.recorder,
            &self.project_root,
            &fresh,
            &self.tool_handle,
            mcp_section,
        )?;
        match self.inner.try_write() {
            Ok(mut guard) => {
                *guard = rebuilt;
            }
            Err(_) => return Err("agent busy, try again".to_string()),
        }
        if let Ok(mut guard) = self.mcp_section.lock() {
            *guard = mcp_section.to_string();
        }
        Ok(())
    }
}

#[async_trait]
impl AgentLoop for RigAgent {
    async fn chat(&self, prompt: &str) -> Result<String, String> {
        const SUMMARIZER_PROMPT: &str = "Summarize this conversation so far for continued work. \
            Preserve: filenames touched, decisions made, errors seen, and any pending tasks. \
            Keep it under 1500 chars. Reply with the summary only.";

        // Lock once for the whole turn: history mutation + compaction are atomic
        // w.r.t. other chat() calls. The TUI serializes turns anyway.
        let ctx = self.context.lock().await;
        let before = ctx.len();
        drop(ctx);

        // Fresh per-turn tool + reasoning logs: the hooks append as the turn runs.
        if let Ok(mut guard) = self.recorder.events.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.recorder.reasoning.lock() {
            guard.clear();
        }

        // Transient provider failures (429/5xx/timeout) retry with backoff;
        // the final error is labeled with the active provider tag.
        // The context lock is per-attempt rather than per-turn: turns are
        // serialized by the TUI, so no other chat() can interleave between
        // attempts. Shared refs are copied out front so the retry closures
        // own everything their futures need (no borrows escape the loop).
        let tag = self.provider_tag();
        let prompt_owned: String = prompt.to_owned();
        let inner_lock = &self.inner;
        let ctx_lock = &self.context;
        let reply = with_retry(move |_| {
            let p = prompt_owned.clone();
            async move {
                let inner = inner_lock.read().await;
                let mut ctx = ctx_lock.lock().await;
                Chat::chat(&*inner, p, ctx.messages_mut())
                    .await
                    .map_err(|e| e.to_string())
            }
        })
        .await
        .map_err(|e| format!("{tag}: {e}"))?;

        // Truncate newly committed tool outputs so they don't eat the window.
        let mut ctx = self.context.lock().await;
        ctx.truncate_new(before);

        // Compact past the trigger: summarize oldest, keep newest verbatim.
        if let Some(drained) = ctx.take_for_compaction() {
            let material = Context::render_for_summary(&drained);
            // Summarize WITHOUT tools and WITHOUT history: a plain one-shot
            // call so the summary can't recurse into compaction or tool loops.
            // Same retry contract as the Run call; unrecoverable failure
            // restores history instead of dropping it.
            let inner_lock = &self.inner;
            let summary = with_retry(move |_| {
                let m = material.clone();
                async move {
                    inner_lock
                        .read()
                        .await
                        .prompt(m)
                        .preamble(SUMMARIZER_PROMPT)
                        .tool_choice(ToolChoice::None)
                        .await
                        .map_err(|e| e.to_string())
                }
            })
            .await;
            match summary {
                Ok(summary) => ctx.apply_summary(summary),
                Err(e) => {
                    // Summarizer failed: put the drained messages back instead
                    // of silently dropping history. Next turn retries.
                    let mut restored = drained;
                    restored.append(ctx.messages_mut());
                    *ctx.messages_mut() = restored;
                    return Err(format!("context compaction failed, history restored: {e}"));
                }
            }
        }

        Ok(reply)
    }

    fn last_tool_events(&self) -> Vec<ToolEvent> {
        self.recorder
            .events
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    fn last_reasoning(&self) -> Vec<String> {
        self.recorder
            .reasoning
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    fn model_name(&self) -> String {
        self.provider_tag()
    }

    fn effort_name(&self) -> String {
        self.switch_state
            .lock()
            .map(|s| s.effort.clone())
            .unwrap_or_else(|_| "medium".to_string())
    }
    fn mode_handle(&self) -> Arc<Mutex<Mode>> {
        self.mode.clone()
    }
    async fn export_messages_json(&self) -> Result<String, String> {
        self.context.lock().await.to_json()
    }
    async fn import_messages_json(&self, json: &str) -> Result<(), String> {
        let ctx = Context::from_json(json)?;
        *self.context.lock().await = ctx;
        Ok(())
    }
    // Sync variants for the sync TUI submit path via try_lock (safe on runtime thread).
    fn export_sync(&self) -> Result<String, String> {
        match self.context.try_lock() {
            Ok(g) => g.to_json(),
            Err(_) => Err("context busy, try again".to_string()),
        }
    }
    fn import_sync(&self, json: &str) -> Result<(), String> {
        let ctx = Context::from_json(json)?;
        match self.context.try_lock() {
            Ok(mut g) => {
                *g = ctx;
                Ok(())
            }
            Err(_) => Err("context busy, try again".to_string()),
        }
    }
    fn list_skills(&self) -> Vec<SkillSummary> {
        self.skills
            .lock()
            .map(|g| {
                g.iter()
                    .map(|s| SkillSummary {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        path: s.path.to_string_lossy().to_string(),
                        note: s.note.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn get_skill_body(&self, name: &str) -> Option<String> {
        self.skills
            .lock()
            .ok()?
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.body.clone())
    }
    fn list_mcp_servers(&self) -> Vec<McpServerSummary> {
        // Sync TUI path into async handle reads: block in place (multi-
        // thread main runtime) so `/mcp` never starts a turn.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(crate::mcp::mcp_server_summaries(
                &self.mcp_state,
                &self.tool_handle,
            ))
        })
    }
    fn mcp_login(&self, name: &str) -> Result<String, String> {
        // Login is async (discovery + registration + browser wait
        // happen in a spawned task): same block-in-place bridge as the
        // toggle. Returns the auth URL; completion reconnects.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(crate::mcp::mcp_login(
                &self.mcp_state,
                &self.tool_handle,
                name,
                &crate::mcp::default_token_store(),
            ))
        })
    }
    fn mcp_logout(&self, name: &str) -> Result<String, String> {
        // Logout is async (drop + tool removal): same bridge. Rebuilds
        // the preamble index like the toggle path.
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(crate::mcp::mcp_logout(
                &self.mcp_state,
                &self.tool_handle,
                name,
                &crate::mcp::default_token_store(),
            ))
        });
        if result.is_ok() {
            self.rebuild_mcp_preamble()?;
        }
        result
    }
    fn set_mcp_server_enabled(&self, name: &str, enabled: bool) -> Result<String, String> {
        // Toggle is async (reconnect does I/O): same block-in-place
        // bridge as `list_mcp_servers`. Refused while busy by the caller.
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(crate::mcp::mcp_set_enabled(
                &self.mcp_state,
                &self.tool_handle,
                name,
                enabled,
            ))
        });
        // Toggle changes the tool set: rebuild the preamble index so the
        // Principal sees current tools. Fail-closed on rebuild error.
        if result.is_ok() {
            self.rebuild_mcp_preamble()?;
        }
        result
    }
    /// Re-scan skill dirs + rebuild the inner `Agent` so subsequent turns see
    /// new/edited skills. History (`Context`), recorder, gate allows, mode,
    /// and switch state survive (same Arcs reused). Fail-closed: rebuild
    /// error keeps old agent.
    /// Caller (TUI) must refuse while a turn is in-flight (`app.busy`).
    /// Reload preserves the MCP preamble index: reload is skills-only.
    /// Ticket 05 recomputes the section on `/mcp` toggles.
    fn reload_skills_sync(&self) -> Result<String, String> {
        let fresh = crate::skills::discover(&self.project_root);
        let state = self
            .switch_state
            .lock()
            .map_err(|_| "agent busy, try again".to_string())?;
        let staged = staged_config(&self.cfg, &state.provider, &state.model, &state.effort)?;
        let effort = state.effort.clone();
        drop(state);
        let section = self
            .mcp_section
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let rebuilt = Self::build_inner(
            &staged,
            &effort,
            &self.gate,
            &self.recorder,
            &self.project_root,
            &fresh,
            &self.tool_handle,
            &section,
        )?;
        match self.inner.try_write() {
            Ok(mut guard) => {
                *guard = rebuilt;
            }
            Err(_) => return Err("agent busy, try again".to_string()),
        }
        let names: Vec<String> = fresh.iter().map(|s| s.name.clone()).collect();
        if let Ok(mut guard) = self.skills.lock() {
            *guard = fresh;
        }
        match names.is_empty() {
            true => Ok("reloaded 0 skills".to_string()),
            false => Ok(format!(
                "reloaded {} skill{}: {}",
                names.len(),
                if names.len() == 1 { "" } else { "s" },
                names.join(", ")
            )),
        }
    }

    /// Switch the active provider/model mid-Session. `spec` is `model`
    /// (same provider) or `provider:model` (cross provider). Rebuilds the
    /// inner `Agent` reusing the Gate, recorder, mode, and skills;
    /// caller-owned history is preserved. Fail-closed: rebuild errors keep
    /// the old agent. Refuses empty specs and unknown providers.
    /// Returns the new `provider:model` tag.
    /// Caller (TUI) must refuse while a turn is in-flight (`app.busy`).
    fn switch_model_sync(&self, spec: &str) -> Result<String, String> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err("usage: /model [provider:]<name>".to_string());
        }
        // Snapshot the live pointer once; the commit below is atomic.
        let (cur_provider, cur_effort) = {
            let state = self
                .switch_state
                .lock()
                .map_err(|_| "agent busy, try again".to_string())?;
            (state.provider.clone(), state.effort.clone())
        };
        let (provider, model) = match spec.split_once(':') {
            Some((p, m)) if !p.trim().is_empty() && !m.trim().is_empty() => {
                (p.trim().to_string(), m.trim().to_string())
            }
            _ => (cur_provider, spec.to_string()),
        };
        if !self.cfg.providers.contains_key(&provider) {
            return Err(format!("unknown provider \"{provider}\""));
        }
        // Stage through the shared startup path so key/endpoint/params
        // resolve identically. Resolve eagerly: a bad key fails before the
        // old agent is touched.
        let staged = staged_config(&self.cfg, &provider, &model, &cur_effort)?;
        staged.active()?;
        let skills_snapshot = self
            .skills
            .lock()
            .map(|g| g.clone())
            .map_err(|_| "agent busy, try again".to_string())?;
        // Preserve the MCP preamble index across model switches.
        let section = self
            .mcp_section
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let rebuilt = Self::build_inner(
            &staged,
            &cur_effort,
            &self.gate,
            &self.recorder,
            &self.project_root,
            &skills_snapshot,
            &self.tool_handle,
            &section,
        )?;
        match self.inner.try_write() {
            Ok(mut guard) => {
                *guard = rebuilt;
            }
            Err(_) => return Err("agent busy, try again".to_string()),
        }
        let mut state = self
            .switch_state
            .lock()
            .map_err(|_| "agent busy, try again".to_string())?;
        state.provider = provider;
        state.model = model;
        Ok(state.tag())
    }

    /// Set the session-local reasoning effort. Accepts `low`/`medium`/`high`
    /// (anything else normalizes to `medium`), rebuilds request params for
    /// the active provider, preserves history. Returns the normalized level.
    /// Caller (TUI) must refuse while a turn is in-flight (`app.busy`).
    fn set_effort_sync(&self, level: &str) -> Result<String, String> {
        let effort = normalize_effort(level.trim()).to_string();
        let (provider, model) = {
            let state = self
                .switch_state
                .lock()
                .map_err(|_| "agent busy, try again".to_string())?;
            (state.provider.clone(), state.model.clone())
        };
        let staged = staged_config(&self.cfg, &provider, &model, &effort)?;
        staged.active()?;
        let skills_snapshot = self
            .skills
            .lock()
            .map(|g| g.clone())
            .map_err(|_| "agent busy, try again".to_string())?;
        // Preserve the MCP preamble index across effort switches.
        let section = self
            .mcp_section
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let rebuilt = Self::build_inner(
            &staged,
            &effort,
            &self.gate,
            &self.recorder,
            &self.project_root,
            &skills_snapshot,
            &self.tool_handle,
            &section,
        )?;
        match self.inner.try_write() {
            Ok(mut guard) => {
                *guard = rebuilt;
            }
            Err(_) => return Err("agent busy, try again".to_string()),
        }
        if let Ok(mut state) = self.switch_state.lock() {
            state.effort = effort.clone();
        }
        Ok(effort)
    }

    /// All configured `provider:model` combos for `/model` listing.
    /// The active provider shows its live model; the rest show defaults.
    fn list_models(&self) -> Vec<String> {
        let mut names: Vec<String> = self.cfg.providers.keys().cloned().collect();
        names.sort();
        let live = self.switch_state.lock().ok().map(|s| s.clone());
        names
            .into_iter()
            .map(|provider| {
                let model = match &live {
                    Some(s) if s.provider == provider => s.model.clone(),
                    _ => self
                        .cfg
                        .providers
                        .get(&provider)
                        .map(|e| e.default_model.clone())
                        .unwrap_or_default(),
                };
                format!("{provider}:{model}")
            })
            .collect()
    }

    /// Local usage snapshot for `/context`, read off the caller-owned
    /// history. Lock contention reports zeros rather than blocking.
    fn context_stats_sync(&self) -> crate::context::ContextStats {
        match self.context.try_lock() {
            Ok(g) => g.stats(),
            Err(_) => crate::context::ContextStats::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tool_server_registers_all_eleven_builtins() {
        let handle = build_tool_server();
        let mut names: Vec<String> = handle
            .get_tool_defs(None)
            .await
            .expect("tool defs")
            .into_iter()
            .map(|d| d.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "bash",
                "edit",
                "git_diff",
                "git_status",
                "glob",
                "grep",
                "list_directory",
                "read",
                "web_fetch",
                "web_search",
                "write",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
    }

    /// Test config in the current multi-provider shape. Keys resolve via
    /// `env:` indirection, so the matching vars must be set (see
    /// `set_test_key_vars`).
    fn mcp_test_config() -> Config {
        toml::from_str(
            r#"
active_provider = "openai"
effort = "medium"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.example.com/v1"
key = "env:REM_TEST_MCP_OPENAI"
default_model = "gpt-4o"
"#,
        )
        .unwrap()
    }

    /// Test-only env setup for [`mcp_test_config`]; idempotent, safe to
    /// call from every MCP test (same values for the process lifetime).
    fn set_test_key_vars() {
        // SAFETY: test-only env mutation with constant values.
        unsafe {
            std::env::set_var("REM_TEST_MCP_OPENAI", "sk-test");
        }
    }

    #[test]
    fn reload_skills_preserves_shared_handle_registrations() {
        use rig::tool::DynamicTool;
        set_test_key_vars();
        let cfg = mcp_test_config();
        let (tx, _rx) = crate::permissions::approval_channel();
        let dir = std::env::temp_dir().join("rem-mcp-test-reload-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let agent = RigAgent::new(&cfg, tx, dir.clone()).expect("agent");
        // Simulate an MCP registration landing on the shared handle.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(
            agent
                .tool_server_handle()
                .add_dynamic_tool(DynamicTool::new(
                    "mcp__py__echo_downcase",
                    "MCP tool",
                    serde_json::json!({"type": "object"}),
                    |_ctx, _args| Box::pin(async move { Ok(rig::tool::ToolOutput::text("hi")) }),
                )),
        );
        agent.reload_skills_sync().expect("reload");
        let names = rt
            .block_on(agent.tool_server_handle().get_tool_defs(None))
            .expect("defs");
        assert!(
            names.iter().any(|d| d.name == "mcp__py__echo_downcase"),
            "reload dropped MCP registration: {:?}",
            names.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_preamble_appends_mcp_section_after_skills() {
        let dir = std::env::temp_dir().join("rem-test-preamble-mcp-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = build_preamble(&dir, &[], "\n\nMCP tools:\n- mcp__py__echo\n");
        assert!(text.contains("mcp__py__echo"), "got: {text}");
        // Static instructions come first; MCP index rides at the end.
        let static_pos = text.find("You are rem").expect("static preamble");
        let mcp_pos = text.find("mcp__py__echo").unwrap();
        assert!(static_pos < mcp_pos);
        // Empty section adds nothing.
        assert!(!build_preamble(&dir, &[], "").contains("MCP"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live bridge coverage: sync trait methods over the async MCP
    /// state. Needs multi_thread (block_in_place panics on
    /// current-thread). Production main is multi-thread.
    #[tokio::test(flavor = "multi_thread")]
    async fn live_mcp_panel_bridge_lists_and_rejects_unknown() {
        set_test_key_vars();
        let cfg = mcp_test_config();
        let (tx, _rx) = crate::permissions::approval_channel();
        let dir = std::env::temp_dir().join("rem-mcp-test-panel-bridge-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let agent = RigAgent::new(&cfg, tx, dir.clone()).expect("agent");
        // Empty state: panel lists nothing, toggle rejects unknown.
        assert!(agent.list_mcp_servers().is_empty());
        let err = agent
            .set_mcp_server_enabled("nope", false)
            .expect_err("unknown server");
        assert!(err.contains("nope"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live bridge coverage: sync login/logout trait methods.
    /// Needs multi_thread (block_in_place panics on current-thread).
    #[tokio::test(flavor = "multi_thread")]
    async fn live_mcp_login_logout_bridge() {
        set_test_key_vars();
        let cfg = mcp_test_config();
        let (tx, _rx) = crate::permissions::approval_channel();
        let dir = std::env::temp_dir().join("rem-mcp-test-login-bridge-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let agent = RigAgent::new(&cfg, tx, dir.clone()).expect("agent");
        // Unknown server: login + logout both error naming it.
        let err = agent.mcp_login("nope").expect_err("unknown login");
        assert!(err.contains("nope"), "got: {err}");
        let err = agent.mcp_logout("nope").expect_err("unknown logout");
        assert!(err.contains("nope"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rate_limit_and_server_errors_are_retryable() {
        // Independent literals: the contract, not the implementation.
        assert!(is_retryable_error("429 Too Many Requests"));
        assert!(is_retryable_error("rate limit exceeded, retry later"));
        assert!(is_retryable_error("503 Service Unavailable"));
        assert!(is_retryable_error("500 Internal Server Error"));
        assert!(is_retryable_error("request timed out after 30s"));
        assert!(is_retryable_error("connection reset by peer"));
    }

    #[test]
    fn auth_and_client_errors_are_not_retryable() {
        assert!(!is_retryable_error("401 Unauthorized: invalid API key"));
        assert!(!is_retryable_error("400 Bad Request: unknown model"));
        assert!(!is_retryable_error(
            "context compaction failed, history restored: boom"
        ));
    }

    #[test]
    fn backoff_schedule_grows_per_attempt_with_fixed_attempt_count() {
        // Independent literals from the ticket-03 contract: 3 attempts,
        // exponential backoff from a 500ms base.
        assert_eq!(MAX_RETRY_ATTEMPTS, 3);
        assert_eq!(retry_delay(0), std::time::Duration::from_millis(500));
        assert_eq!(retry_delay(1), std::time::Duration::from_millis(1000));
        assert_eq!(retry_delay(2), std::time::Duration::from_millis(2000));
    }

    #[tokio::test]
    async fn transient_failures_retry_then_succeed() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let out = with_retry(|_| async {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            match n < 2 {
                true => Err::<&str, String>("503 Service Unavailable".to_string()),
                false => Ok("done"),
            }
        })
        .await;
        assert_eq!(out, Ok("done"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn persistent_transient_failure_returns_last_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let out = with_retry(|_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<&str, String>("429 Too Many Requests".to_string())
        })
        .await;
        assert_eq!(out, Err("429 Too Many Requests".to_string()));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn non_retryable_error_fails_fast_without_retry() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let out = with_retry(|_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<&str, String>("401 Unauthorized".to_string())
        })
        .await;
        assert_eq!(out, Err("401 Unauthorized".to_string()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn effort_mapping_keeps_unified_scale_across_providers() {
        // Independent literals from the ticket-02 effort table.
        assert_eq!(normalize_effort("low"), "low");
        assert_eq!(normalize_effort("medium"), "medium");
        assert_eq!(normalize_effort("high"), "high");
        assert_eq!(normalize_effort(""), "medium");
        assert_eq!(normalize_effort("ultra"), "medium");
    }

    #[test]
    fn anthropic_budget_grows_with_effort_and_max_exceeds_budget() {
        let (low_budget, low_max) = anthropic_budget("low");
        let (med_budget, med_max) = anthropic_budget("medium");
        let (high_budget, high_max) = anthropic_budget("high");
        assert!(low_budget < med_budget && med_budget < high_budget);
        for (budget, max) in [
            (low_budget, low_max),
            (med_budget, med_max),
            (high_budget, high_max),
        ] {
            assert!(max > budget, "max {max} must exceed budget {budget}");
        }
    }

    fn test_config() -> Config {
        toml::from_str(
            r#"
active_provider = "openai"
effort = "medium"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.example.com/v1"
key = "env:REM_TEST_SWITCH_OPENAI"
default_model = "gpt-4o"

[providers.anthropic]
type = "anthropic"
key = "env:REM_TEST_SWITCH_ANTHROPIC"
default_model = "claude-sonnet-4-5"
"#,
        )
        .unwrap()
    }

    fn test_agent() -> RigAgent {
        // SAFETY: test-only env mutation; all switch tests set the same
        // values, and rebuilds re-resolve keys on every switch/effort call,
        // so the vars stay set for the test process lifetime.
        unsafe {
            std::env::set_var("REM_TEST_SWITCH_OPENAI", "sk-openai-test");
            std::env::set_var("REM_TEST_SWITCH_ANTHROPIC", "sk-anthropic-test");
        }
        let dir = std::env::temp_dir().join("rem-test-switch-agent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = crate::permissions::approval_channel();
        RigAgent::new(&test_config(), tx, dir).unwrap()
    }

    #[test]
    fn switch_same_provider_updates_live_model_tag() {
        let agent = test_agent();
        assert_eq!(agent.model_name(), "openai:gpt-4o");
        let tag = agent.switch_model_sync("gpt-4o-mini").unwrap();
        assert_eq!(tag, "openai:gpt-4o-mini");
        assert_eq!(agent.model_name(), "openai:gpt-4o-mini");
    }

    #[test]
    fn switch_cross_provider_updates_tag() {
        let agent = test_agent();
        let tag = agent
            .switch_model_sync("anthropic:claude-haiku-4-5")
            .unwrap();
        assert_eq!(tag, "anthropic:claude-haiku-4-5");
        assert_eq!(agent.model_name(), "anthropic:claude-haiku-4-5");
    }

    #[test]
    fn switch_unknown_provider_fails_without_touching_live_tag() {
        let agent = test_agent();
        let err = agent
            .switch_model_sync("nope:model")
            .expect_err("must fail");
        assert!(err.contains("unknown provider"), "got: {err}");
        assert_eq!(agent.model_name(), "openai:gpt-4o");
    }

    #[test]
    fn switch_empty_spec_reports_usage_without_touching_live_tag() {
        let agent = test_agent();
        let err = agent.switch_model_sync("  ").expect_err("must fail");
        assert!(err.contains("/model"), "got: {err}");
        assert_eq!(agent.model_name(), "openai:gpt-4o");
    }

    #[test]
    fn set_effort_rebuilds_and_returns_normalized_level() {
        let agent = test_agent();
        assert_eq!(agent.set_effort_sync("high").unwrap(), "high");
        assert_eq!(agent.effort_name(), "high");
        // Unrecognized levels normalize to medium.
        assert_eq!(agent.set_effort_sync("ultra").unwrap(), "medium");
        assert_eq!(agent.effort_name(), "medium");
    }

    #[test]
    fn list_models_shows_live_model_for_active_provider() {
        let agent = test_agent();
        assert_eq!(
            agent.list_models(),
            vec![
                "anthropic:claude-sonnet-4-5".to_string(),
                "openai:gpt-4o".to_string(),
            ]
        );
        agent.switch_model_sync("gpt-4o-mini").unwrap();
        assert_eq!(
            agent.list_models(),
            vec![
                "anthropic:claude-sonnet-4-5".to_string(),
                "openai:gpt-4o-mini".to_string(),
            ]
        );
    }

    #[test]
    fn context_stats_reflects_live_history() {
        let agent = test_agent();
        let stats = agent.context_stats_sync();
        assert_eq!(stats.messages, 0);
        assert_eq!(stats.trigger, crate::context::Context::TRIGGER);
        agent
            .import_sync(&crate::context::Context::new().to_json().unwrap())
            .unwrap();
        let mut ctx = crate::context::Context::new();
        ctx.messages_mut()
            .push(rig::message::Message::user("hello world"));
        agent.import_sync(&ctx.to_json().unwrap()).unwrap();
        let stats = agent.context_stats_sync();
        assert_eq!(stats.messages, 1);
        assert_eq!(stats.chars, "hello world".len());
        assert!(!stats.summarized);
    }

    #[test]
    fn switch_preserves_caller_owned_history() {
        let agent = test_agent();
        // Seed history through the public import surface.
        let mut ctx = crate::context::Context::new();
        ctx.messages_mut()
            .push(rig::message::Message::user("remember the plan"));
        let before = ctx.to_json().unwrap();
        agent.import_sync(&before).unwrap();
        // Switch provider and model; history must survive verbatim.
        agent
            .switch_model_sync("anthropic:claude-haiku-4-5")
            .unwrap();
        assert_eq!(agent.export_sync().unwrap(), before);
        // Effort changes preserve history too.
        agent.set_effort_sync("high").unwrap();
        assert_eq!(agent.export_sync().unwrap(), before);
    }

    #[test]
    fn load_agents_md_returns_none_when_missing() {
        let dir = std::env::temp_dir().join("rem-test-no-agents-md-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_agents_md(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_skills_sync_preserves_list_shape() {
        // Shape test without a live LLM client: build_inner needs real creds,
        // so exercise the summary mapping via discover_in + preamble only.
        let base = std::env::temp_dir().join("rem-skill-test-reload-shape");
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("skills/pdf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\ndescription: D.\n---\nBody.\n").unwrap();
        let found = crate::skills::discover_in(&[base.join("skills")]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "pdf");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn load_agents_md_returns_verbatim_body() {
        let dir = std::env::temp_dir().join("rem-test-agents-md-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "# rules\nUse tabs.\n").unwrap();
        assert_eq!(
            load_agents_md(&dir).as_deref(),
            Some("# rules\nUse tabs.\n")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
