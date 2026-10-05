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
    providers::openai,
};

use crate::{
    config::Config,
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
    model_name: String,
    effort: String,
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

/// Project instructions file name, loaded from the project root only.
const AGENTS_MD: &str = "AGENTS.md";
/// Header separating static instructions from project context in the preamble.
const AGENTS_MD_HEADER: &str = "\n\nProject context from AGENTS.md:\n";

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

        let agent = Self::build_inner(
            cfg,
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
            model_name: cfg.model.default.clone(),
            effort: cfg.model.effort.clone(),
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
    /// called at startup and on `/skills reload`.
    fn build_inner(
        cfg: &Config,
        gate: &PermissionHook,
        recorder: &ToolRecorder,
        project_root: &std::path::Path,
        skills: &[crate::skills::Skill],
        tool_handle: &rig::agent::tool::server::ToolServerHandle,
        mcp_section: &str,
    ) -> Result<rig::agent::Agent, String> {
        // Explicit `.base_url()` per spec — no reliance on OPENAI_BASE_URL env.
        let client = openai::CompletionsClient::builder()
            .api_key(cfg.api.key.clone())
            .base_url(cfg.api.base_url.clone())
            .build()
            .map_err(|e| format!("failed to build LLM client: {e}"))?;

        let model = client.completion_model(cfg.model.default.clone());

        let preamble = build_preamble(project_root, skills, mcp_section);

        let agent = AgentBuilder::new(model)
            .preamble(&preamble)
            .tool_server_handle(tool_handle.clone())
            .default_max_turns(100)
            .additional_params(serde_json::json!({"reasoning_effort": cfg.model.effort.clone()}))
            .add_hook(recorder.clone())
            .add_hook(gate.clone())
            .build();

        Ok(agent)
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
        let rebuilt = Self::build_inner(
            &self.cfg,
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
        let rebuilt = Self::build_inner(
            &self.cfg,
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
        let mut ctx = self.context.lock().await;
        let before = ctx.len();

        // Fresh per-turn tool + reasoning logs: the hooks append as the turn runs.
        if let Ok(mut guard) = self.recorder.events.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.recorder.reasoning.lock() {
            guard.clear();
        }

        // Read-lock the inner agent for the turn. Safe: hooks never lock
        // `inner` or `context`, so no lock cycle with `ctx` held.
        let inner = self.inner.read().await;
        let reply = Chat::chat(&*inner, prompt, ctx.messages_mut())
            .await
            .map_err(|e| e.to_string())?;
        drop(inner);

        // Truncate newly committed tool outputs so they don't eat the window.
        ctx.truncate_new(before);

        // Compact past the trigger: summarize oldest, keep newest verbatim.
        if let Some(drained) = ctx.take_for_compaction() {
            let material = Context::render_for_summary(&drained);
            // Summarize WITHOUT tools and WITHOUT history: a plain one-shot
            // call so the summary can't recurse into compaction or tool loops.
            match self
                .inner
                .read()
                .await
                .prompt(material)
                .preamble(SUMMARIZER_PROMPT)
                .tool_choice(ToolChoice::None)
                .await
            {
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
        self.model_name.clone()
    }

    fn effort_name(&self) -> String {
        self.effort.clone()
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
    /// new/edited skills. History (`Context`), recorder, gate allows, and mode
    /// survive (same Arcs reused). Fail-closed: rebuild error keeps old agent.
    /// Caller (TUI) must refuse while a turn is in-flight (`app.busy`).
    /// Reload preserves the MCP preamble index: reload is skills-only.
    /// Ticket 05 recomputes the section on `/mcp` toggles.
    fn reload_skills_sync(&self) -> Result<String, String> {
        let fresh = crate::skills::discover(&self.project_root);
        let section = self
            .mcp_section
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let rebuilt = Self::build_inner(
            &self.cfg,
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

    #[test]
    fn reload_skills_preserves_shared_handle_registrations() {
        use rig::tool::DynamicTool;
        let cfg = Config {
            api: crate::config::ApiConfig {
                key: "sk-test".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
            },
            model: crate::config::ModelConfig {
                default: "gpt-4o".to_string(),
                effort: "medium".to_string(),
            },
            theme: Default::default(),
        };
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
        let cfg = Config {
            api: crate::config::ApiConfig {
                key: "sk-test".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
            },
            model: crate::config::ModelConfig {
                default: "gpt-4o".to_string(),
                effort: "medium".to_string(),
            },
            theme: Default::default(),
        };
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
        let cfg = Config {
            api: crate::config::ApiConfig {
                key: "sk-test".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
            },
            model: crate::config::ModelConfig {
                default: "gpt-4o".to_string(),
                effort: "medium".to_string(),
            },
            theme: Default::default(),
        };
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
