use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

use super::ToolError;

/// One fan-out child: short transcript label + full scoped prompt.
#[derive(Debug, Clone, Deserialize)]
pub struct Subtask {
    pub description: String,
    pub prompt: String,
}

/// `task` args: exactly one shape — single (`description` + `prompt`) or
/// fan-out (`tasks[]`) — plus an optional `background` flag that detaches
/// the whole call. All work fields are optional so the v1 single shape
/// keeps parsing; `call()` enforces exactly-one-shape with model-visible
/// errors (JSON Schema cannot express XOR, same as blank-rejection today).
#[derive(Debug, Deserialize)]
pub struct TaskArgs {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub tasks: Option<Vec<Subtask>>,
    #[serde(default)]
    pub background: bool,
}

/// Max concurrent children per fan-out call; excess queues on the semaphore.
pub const FANOUT_CONCURRENCY: usize = 4;
/// Hard cap on children per call; beyond this the model splits the batch.
pub const FANOUT_MAX_TASKS: usize = 8;

/// Lifecycle of one detached background task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundStatus {
    Running,
    Done,
}

/// Snapshot of one background task for `/tasks` + submit-time injection.
#[derive(Debug, Clone)]
pub struct BackgroundInfo {
    pub id: String,
    pub labels: Vec<String>,
    pub status: BackgroundStatus,
    pub summary: Option<String>,
}

#[derive(Debug)]
struct BackgroundEntry {
    labels: Vec<String>,
    status: BackgroundStatus,
    summary: Option<String>,
    notified: bool,
}

/// Session-scoped store for detached background tasks. Agent-owned
/// (`Arc`, same shape as `mode`); `/new` clears it alongside todos.
/// Summaries are truncated to [`crate::agent::FULL_OUTPUT_CAP`] chars so a
/// runaway child can't grow session memory.
#[derive(Debug, Default)]
pub struct BackgroundStore {
    enabled: Mutex<bool>,
    next: AtomicU64,
    tasks: Mutex<HashMap<String, BackgroundEntry>>,
}

impl BackgroundStore {
    pub fn new() -> Self {
        Self {
            enabled: Mutex::new(true),
            next: AtomicU64::new(0),
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Headless `-p` flips this off: `background:true` then runs inline
    /// (serial result, no polling) instead of detaching.
    pub fn set_enabled(&self, on: bool) {
        if let Ok(mut g) = self.enabled.lock() {
            *g = on;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.lock().map(|g| *g).unwrap_or(false)
    }

    /// Register a detached run; returns the `task-N` id.
    pub fn start(&self, labels: Vec<String>) -> String {
        let n = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("task-{n}");
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.insert(
                id.clone(),
                BackgroundEntry {
                    labels,
                    status: BackgroundStatus::Running,
                    summary: None,
                    notified: false,
                },
            );
        }
        id
    }

    pub fn finish(&self, id: &str, summary: String) {
        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(e) = tasks.get_mut(id)
        {
            e.status = BackgroundStatus::Done;
            e.summary = Some(truncate_chars(&summary, crate::agent::FULL_OUTPUT_CAP));
        }
    }

    pub fn list(&self) -> Vec<BackgroundInfo> {
        let mut out: Vec<BackgroundInfo> = self
            .tasks
            .lock()
            .map(|g| {
                g.iter()
                    .map(|(id, e)| BackgroundInfo {
                        id: id.clone(),
                        labels: e.labels.clone(),
                        status: e.status,
                        summary: e.summary.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn detail(&self, id: &str) -> Option<BackgroundInfo> {
        self.tasks.lock().ok().and_then(|g| {
            g.get(id).map(|e| BackgroundInfo {
                id: id.to_string(),
                labels: e.labels.clone(),
                status: e.status,
                summary: e.summary.clone(),
            })
        })
    }

    /// Drain finished tasks the model hasn't seen yet; marks them notified.
    /// The TUI prepends these to the next submit prompt.
    pub fn take_completed(&self) -> Vec<BackgroundInfo> {
        let mut out = Vec::new();
        if let Ok(mut tasks) = self.tasks.lock() {
            let mut ids: Vec<String> = tasks.keys().cloned().collect();
            ids.sort();
            for id in ids {
                if let Some(e) = tasks.get_mut(&id)
                    && e.status == BackgroundStatus::Done
                    && !e.notified
                {
                    e.notified = true;
                    out.push(BackgroundInfo {
                        id: id.clone(),
                        labels: e.labels.clone(),
                        status: e.status,
                        summary: e.summary.clone(),
                    });
                }
            }
        }
        out
    }

    pub fn clear(&self) {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.clear();
        }
        self.next.store(0, Ordering::SeqCst);
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

/// Child-runner seam. Production (`agent.rs`) runs a fresh inner agent to
/// completion; tests inject a fake. Inline futures are awaited (never
/// detached) so parent-turn abort propagation holds: aborting the outer
/// chat task cancels the in-flight child future. Only `background:true`
/// detaches via `tokio::spawn`, and only into the session store.
#[async_trait]
pub trait TaskChildRunner: Send + Sync {
    async fn run_child(&self, description: &str, prompt: &str) -> Result<String, String>;
}
pub struct TaskTool<R: TaskChildRunner> {
    runner: Arc<R>,
    background: Arc<BackgroundStore>,
}

impl<R: TaskChildRunner> TaskTool<R> {
    pub fn with_background(runner: Arc<R>, store: Arc<BackgroundStore>) -> Self {
        Self {
            runner,
            background: store,
        }
    }
}

impl<R: TaskChildRunner> Clone for TaskTool<R> {
    fn clone(&self) -> Self {
        Self {
            runner: self.runner.clone(),
            background: self.background.clone(),
        }
    }
}

impl<R: TaskChildRunner> std::fmt::Debug for TaskTool<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTool").finish_non_exhaustive()
    }
}

/// Short transcript label for the `task` tree leaf: the single
/// `description`, or `N subtasks: a, b, …` for fan-out. Falls back to the
/// compact-args shape when nothing parses. A `background:true` call gains
/// a `[background]` suffix.
pub fn task_preview(args: &serde_json::Value) -> String {
    let base = match args.get("tasks").and_then(|v| v.as_array()) {
        Some(items) => {
            let labels: Vec<String> = items
                .iter()
                .filter_map(|t| t.get("description").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let shown = match labels.len() {
                0 => "?".to_string(),
                1..=3 => labels.join(", "),
                _ => format!("{}, +{} more", labels[..3].join(", "), labels.len() - 3),
            };
            format!("{} subtasks: {shown}", items.len())
        }
        None => match args.get("description").and_then(|v| v.as_str()) {
            Some(d) if !d.trim().is_empty() => d.to_string(),
            _ => {
                const N: usize = 80;
                let s = args.to_string();
                match s.chars().count() > N {
                    true => format!("{}…", s.chars().take(N - 1).collect::<String>()),
                    false => s,
                }
            }
        },
    };
    match args.get("background").and_then(|v| v.as_bool()) {
        Some(true) => format!("{base} [background]"),
        _ => base,
    }
}

/// Validated work: exactly one shape.
enum Work {
    Single(Subtask),
    Fanout(Vec<Subtask>),
}

/// Validate args into exactly one work list: single or fan-out.
fn normalize(args: &TaskArgs) -> Result<Work, ToolError> {
    let d = args.description.as_deref().map(str::trim).unwrap_or("");
    let p = args.prompt.as_deref().map(str::trim).unwrap_or("");
    match &args.tasks {
        Some(items) => {
            if !d.is_empty() || !p.is_empty() {
                return Err(ToolError(
                    "task: pass either `description`+`prompt` (single) or `tasks[]` (fan-out), not both"
                        .to_string(),
                ));
            }
            if items.is_empty() {
                return Err(ToolError("task: `tasks[]` must not be empty".to_string()));
            }
            if items.len() > FANOUT_MAX_TASKS {
                return Err(ToolError(format!(
                    "task: fan-out capped at {FANOUT_MAX_TASKS} subtasks, got {} — split into smaller batches",
                    items.len()
                )));
            }
            for (i, t) in items.iter().enumerate() {
                if t.description.trim().is_empty() {
                    return Err(ToolError(format!(
                        "task: tasks[{i}]: `description` must not be empty"
                    )));
                }
                if t.prompt.trim().is_empty() {
                    return Err(ToolError(format!(
                        "task: tasks[{i}]: `prompt` must not be empty"
                    )));
                }
            }
            Ok(Work::Fanout(items.clone()))
        }
        None => {
            if d.is_empty() && p.is_empty() {
                return Err(ToolError(
                    "task: pass `description`+`prompt` (single) or `tasks[]` (fan-out)".to_string(),
                ));
            }
            if d.is_empty() {
                return Err(ToolError(
                    "task: `description` must not be empty".to_string(),
                ));
            }
            if p.is_empty() {
                return Err(ToolError("task: `prompt` must not be empty".to_string()));
            }
            Ok(Work::Single(Subtask {
                description: d.to_string(),
                prompt: p.to_string(),
            }))
        }
    }
}

/// Run a fan-out batch bounded-parallel on a scoped `JoinSet` and merge
/// `## <description>` blocks in input order with per-child error blocks
/// (partial merge — one child failing never fails the batch). The set is
/// scoped inside this future: dropping the tool future aborts every child,
/// so Esc keeps the serial abort story with zero TUI work.
async fn run_fanout<R: TaskChildRunner + 'static>(runner: &Arc<R>, items: &[Subtask]) -> String {
    let sem = Arc::new(tokio::sync::Semaphore::new(FANOUT_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();
    for (i, t) in items.iter().enumerate() {
        let runner = runner.clone();
        let sem = sem.clone();
        let (desc, prompt) = (t.description.clone(), t.prompt.clone());
        set.spawn(async move {
            let _permit = sem
                .acquire_owned()
                .await
                .map_err(|e| (i, desc.clone(), e.to_string()))?;
            runner
                .run_child(&desc, &prompt)
                .await
                .map(|s| (i, desc.clone(), s))
                .map_err(|e| (i, desc.clone(), e))
        });
    }
    let mut blocks: Vec<Option<String>> = (0..items.len()).map(|_| None).collect();
    let mut strays: Vec<String> = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok((i, d, s))) => blocks[i] = Some(format!("## {d}\n{s}")),
            Ok(Err((i, d, e))) => blocks[i] = Some(format!("## {d} (error)\n{e}")),
            Err(e) => {
                // Join failure (panic/abort inside the scoped set): attribute
                // to no child; append once so the merge never silently drops.
                strays.push(format!("## (error)\nworker failed: {e}"));
            }
        }
    }
    let mut out: Vec<String> = blocks.into_iter().flatten().collect();
    out.extend(strays);
    out.join("\n\n")
}

impl<R: TaskChildRunner + 'static> Tool for TaskTool<R> {
    const NAME: &'static str = "task";
    type Args = TaskArgs;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Spawn scoped subagent(s) and wait for the merged result. Single: description (short label) + prompt (full task). Fan-out: tasks[] array (up to 8, 4 run at a time) merged as ## sections in input order, per-child errors kept inline. background:true detaches the whole call and returns an id immediately; the summary arrives attached to a later turn (/tasks shows status). Children get fresh context, the same file/shell/git/search/web tools plus todowrite (never task: no recursion).".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "Short transcript label for this subtask (single shape)" },
                "prompt": { "type": "string", "description": "Full scoped task for the subagent (single shape)" },
                "tasks": {
                    "type": "array",
                    "description": "Fan-out batch: up to 8 {description, prompt} subtasks, 4 run at a time",
                    "items": {
                        "type": "object",
                        "properties": {
                            "description": { "type": "string" },
                            "prompt": { "type": "string" }
                        },
                        "required": ["description", "prompt"]
                    }
                },
                "background": { "type": "boolean", "description": "Detach: return an id now, deliver the summary on a later turn" }
            },
            "required": []
        })
    }

    async fn call(&self, _ctx: &mut ToolContext, args: Self::Args) -> Result<String, ToolError> {
        let background = args.background;
        let work = normalize(&args)?;
        if background {
            let store = &self.background;
            if store.is_enabled() {
                let single = matches!(work, Work::Single(_));
                let items: Vec<Subtask> = match work {
                    Work::Single(t) => vec![t],
                    Work::Fanout(ts) => ts,
                };
                let labels: Vec<String> = items.iter().map(|t| t.description.clone()).collect();
                let id = store.start(labels.clone());
                let live_id = id.clone();
                let runner = self.runner.clone();
                let store = Arc::clone(store);
                let n = items.len();
                tokio::spawn(async move {
                    let out = match single {
                        true => {
                            let t = &items[0];
                            runner
                                .run_child(&t.description, &t.prompt)
                                .await
                                .unwrap_or_else(|e| format!("error: {e}"))
                        }
                        false => run_fanout(&runner, &items).await,
                    };
                    store.finish(&live_id, out);
                });
                return Ok(format!(
                    "background {id} started ({n} subagent(s): {}); the merged summary will arrive attached to a later turn",
                    labels.join(", ")
                ));
            }
            // Store disabled (headless `-p`): fall through to inline so the
            // model gets the same result with no polling.
        }
        match work {
            // Single keeps the exact v1 behavior: raw summary, child errors
            // propagate as tool errors.
            Work::Single(t) => self
                .runner
                .run_child(&t.description, &t.prompt)
                .await
                .map_err(ToolError),
            Work::Fanout(items) => Ok(run_fanout(&self.runner, &items).await),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    /// Fake child runner: records the prompt it received, returns a canned
    /// summary. No LLM, no I/O.
    #[derive(Debug, Default)]
    struct FakeRunner {
        last: Mutex<Option<(String, String)>>,
        summary: String,
    }

    impl FakeRunner {
        fn with_summary(summary: &str) -> Self {
            Self {
                last: Mutex::new(None),
                summary: summary.to_string(),
            }
        }
    }

    #[async_trait]
    impl TaskChildRunner for FakeRunner {
        async fn run_child(&self, description: &str, prompt: &str) -> Result<String, String> {
            *self.last.lock().unwrap() = Some((description.to_string(), prompt.to_string()));
            Ok(self.summary.clone())
        }
    }

    /// Echo runner: returns `done-<description>` so merge-order tests can
    /// attribute each block to its child.
    #[derive(Debug, Default)]
    struct EchoRunner;

    #[async_trait]
    impl TaskChildRunner for EchoRunner {
        async fn run_child(&self, description: &str, _prompt: &str) -> Result<String, String> {
            Ok(format!("done-{description}"))
        }
    }

    fn tool_with(summary: &str) -> (TaskTool<FakeRunner>, Arc<FakeRunner>) {
        let runner = Arc::new(FakeRunner::with_summary(summary));
        let store = Arc::new(BackgroundStore::new());
        (TaskTool::with_background(runner.clone(), store), runner)
    }

    fn tool_with_runner<R: TaskChildRunner + 'static>(runner: Arc<R>) -> TaskTool<R> {
        TaskTool::with_background(runner, Arc::new(BackgroundStore::new()))
    }

    fn single_args(d: &str, p: &str) -> TaskArgs {
        serde_json::from_value(serde_json::json!({"description": d, "prompt": p})).unwrap()
    }

    fn fanout_args(items: &[(&str, &str)]) -> TaskArgs {
        let tasks: Vec<serde_json::Value> = items
            .iter()
            .map(|(d, p)| serde_json::json!({"description": d, "prompt": p}))
            .collect();
        serde_json::from_value(serde_json::json!({"tasks": tasks})).unwrap()
    }

    #[test]
    fn task_args_require_description_and_prompt() {
        let args: TaskArgs = serde_json::from_value(serde_json::json!({
            "description": "fix login",
            "prompt": "Fix the login retry bug in src/auth.rs and summarize."
        }))
        .unwrap();
        assert_eq!(args.description.as_deref(), Some("fix login"));
        assert!(args.prompt.as_deref().unwrap().contains("login retry"));
    }

    #[tokio::test]
    async fn task_args_reject_missing_prompt() {
        // Shapes parse leniently now (compat); validation rejects at call.
        let (tool, _) = tool_with("x");
        let mut ctx = ToolContext::new();
        let args: TaskArgs =
            serde_json::from_value(serde_json::json!({"description": "fix login"})).unwrap();
        assert!(tool.call(&mut ctx, args).await.is_err());
        let args: TaskArgs = serde_json::from_value(serde_json::json!({"prompt": "do x"})).unwrap();
        assert!(tool.call(&mut ctx, args).await.is_err());
    }

    #[tokio::test]
    async fn task_call_returns_child_summary_and_records_prompt() {
        let (tool, runner) = tool_with("fixed it");
        let mut ctx = ToolContext::new();
        let args = single_args("fix login", "Fix the login retry bug.");
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert_eq!(out, "fixed it");
        let (d, p) = runner.last.lock().unwrap().clone().unwrap();
        assert_eq!(d, "fix login");
        assert_eq!(p, "Fix the login retry bug.");
    }

    #[tokio::test]
    async fn task_call_rejects_blank_prompt_and_description() {
        let (tool, _) = tool_with("x");
        let mut ctx = ToolContext::new();
        let args = single_args("fix login", "   ");
        assert!(tool.call(&mut ctx, args).await.is_err());
        let args = single_args("", "do x");
        assert!(tool.call(&mut ctx, args).await.is_err());
    }

    #[tokio::test]
    async fn task_call_surfaces_child_failure() {
        #[derive(Debug)]
        struct FailRunner;
        #[async_trait]
        impl TaskChildRunner for FailRunner {
            async fn run_child(&self, _d: &str, _p: &str) -> Result<String, String> {
                Err("child blew up".to_string())
            }
        }
        let tool = tool_with_runner(Arc::new(FailRunner));
        let mut ctx = ToolContext::new();
        let args = single_args("d", "p");
        let err = tool.call(&mut ctx, args).await.expect_err("must fail");
        assert!(err.0.contains("child blew up"), "got: {err}");
    }

    #[test]
    fn task_preview_uses_description() {
        assert_eq!(
            task_preview(&serde_json::json!({"description": "fix login", "prompt": "x"})),
            "fix login"
        );
        // Missing description falls back to compact args, never empty.
        let fallback = task_preview(&serde_json::json!({"prompt": "x"}));
        assert!(fallback.contains("prompt"), "got: {fallback}");
    }

    #[tokio::test]
    async fn fanout_merges_in_input_order() {
        let tool = tool_with_runner(Arc::new(EchoRunner));
        let mut ctx = ToolContext::new();
        let args = fanout_args(&[
            ("b-one", "do one"),
            ("a-two", "do two"),
            ("c-three", "do three"),
        ]);
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert_eq!(
            out,
            "## b-one\ndone-b-one\n\n## a-two\ndone-a-two\n\n## c-three\ndone-c-three"
        );
    }

    #[tokio::test]
    async fn fanout_partial_merge_keeps_child_errors_inline() {
        #[derive(Debug)]
        struct FlakyRunner;
        #[async_trait]
        impl TaskChildRunner for FlakyRunner {
            async fn run_child(&self, description: &str, _p: &str) -> Result<String, String> {
                match description {
                    "bad" => Err("blew up".to_string()),
                    d => Ok(format!("ok-{d}")),
                }
            }
        }
        let tool = tool_with_runner(Arc::new(FlakyRunner));
        let mut ctx = ToolContext::new();
        let args = fanout_args(&[("good", "g"), ("bad", "b")]);
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert_eq!(out, "## good\nok-good\n\n## bad (error)\nblew up");
    }

    #[tokio::test]
    async fn fanout_rejects_over_cap_and_empty_and_both_shapes() {
        let tool = tool_with_runner(Arc::new(EchoRunner));
        let mut ctx = ToolContext::new();
        let big: Vec<(String, String)> =
            (0..9).map(|i| (format!("d{i}"), "p".to_string())).collect();
        let big_refs: Vec<(&str, &str)> =
            big.iter().map(|(d, p)| (d.as_str(), p.as_str())).collect();
        let err = tool
            .call(&mut ctx, fanout_args(&big_refs))
            .await
            .expect_err("9 > cap");
        assert!(err.0.contains("capped"), "got: {err}");
        let err = tool
            .call(&mut ctx, fanout_args(&[]))
            .await
            .expect_err("empty");
        assert!(err.0.contains("must not be empty"), "got: {err}");
        let args: TaskArgs = serde_json::from_value(serde_json::json!({
            "description": "d", "prompt": "p",
            "tasks": [{"description": "x", "prompt": "y"}]
        }))
        .unwrap();
        let err = tool.call(&mut ctx, args).await.expect_err("both shapes");
        assert!(err.0.contains("not both"), "got: {err}");
    }

    #[tokio::test]
    async fn fanout_five_children_all_complete() {
        let tool = tool_with_runner(Arc::new(EchoRunner));
        let mut ctx = ToolContext::new();
        let args = fanout_args(&[
            ("one", "1"),
            ("two", "2"),
            ("three", "3"),
            ("four", "4"),
            ("five", "5"),
        ]);
        let out = tool.call(&mut ctx, args).await.unwrap();
        for name in ["one", "two", "three", "four", "five"] {
            assert!(
                out.contains(&format!("## {name}\ndone-{name}")),
                "got: {out}"
            );
        }
    }

    #[test]
    fn task_preview_fanout_and_background() {
        let p = task_preview(&serde_json::json!({
            "tasks": [
                {"description": "a", "prompt": "x"},
                {"description": "b", "prompt": "y"},
            ]
        }));
        assert_eq!(p, "2 subtasks: a, b");
        let p = task_preview(&serde_json::json!({
            "description": "fix login", "prompt": "x", "background": true
        }));
        assert_eq!(p, "fix login [background]");
    }

    #[tokio::test]
    async fn background_detaches_and_store_completes() {
        let store = Arc::new(BackgroundStore::new());
        let tool = TaskTool::with_background(Arc::new(EchoRunner), store.clone());
        let mut ctx = ToolContext::new();
        let mut args = single_args("fix login", "Fix it.");
        args.background = true;
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert!(out.contains("task-1"), "got: {out}");
        assert!(out.contains("background"), "got: {out}");
        let mut info = None;
        for _ in 0..100 {
            if let Some(i) = store.detail("task-1")
                && i.status == BackgroundStatus::Done
            {
                info = Some(i);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let info = info.expect("background task completes");
        assert!(info.summary.as_deref().unwrap().contains("done-fix login"));
        // Drained exactly once by the TUI injection path.
        assert_eq!(store.take_completed().len(), 1);
        assert!(store.take_completed().is_empty());
    }

    #[tokio::test]
    async fn background_disabled_runs_inline() {
        let store = Arc::new(BackgroundStore::new());
        store.set_enabled(false);
        let tool = TaskTool::with_background(Arc::new(EchoRunner), store.clone());
        let mut ctx = ToolContext::new();
        let mut args = single_args("fix login", "Fix it.");
        args.background = true;
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert_eq!(out, "done-fix login");
        assert!(store.list().is_empty());
    }
}
