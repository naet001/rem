use rig::message::{AssistantContent, Message, ToolResultContent, UserContent};

/// Local usage snapshot for `/context`. Plain data, no Rig internals —
/// the TUI renders it without touching message shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextStats {
    pub messages: usize,
    pub chars: usize,
    pub trigger: usize,
    pub keep: usize,
    pub summarized: bool,
}

/// Marker prefix for compaction summaries (see `apply_summary`).
const SUMMARY_PREFIX: &str = "Prior conversation summary:";

/// Hard cap on stored checkpoints: the oldest snapshot is dropped past this.
pub const MAX_CHECKPOINTS: usize = 50;

/// Checkpoint captures the state at a particular turn for undo/rewind support.
/// A snapshot of `git diff` + `messages_json` + turn index, stored in a bounded log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub git_diff: String,
    pub messages_json: String,
    pub turn_idx: usize,
}

impl Checkpoint {
    /// Snapshot the current messages plus the caller's `git diff` output.
    /// `messages_json` serializes the full history so restore replaces it verbatim.
    /// Test-only helper: production captures through [`Context::record_turn`],
    /// which receives the pre-turn strings from the agent loop.
    #[cfg(test)]
    pub fn new_checkpoint(ctx: &Context, turn_idx: usize, git_diff: &str) -> Self {
        let messages_json = ctx.to_json().unwrap_or_default();
        Self {
            git_diff: git_diff.to_string(),
            messages_json,
            turn_idx,
        }
    }

    /// Drop the oldest entries until `checkpoints.len() <= max`.
    /// Returns the remaining oldest-first order so callers can assert on it.
    pub fn prune_checkpoints(
        checkpoints: &mut std::collections::VecDeque<Checkpoint>,
        max: usize,
    ) -> Vec<Checkpoint> {
        while checkpoints.len() > max {
            checkpoints.pop_front();
        }
        checkpoints.iter().cloned().collect()
    }
}

/// True when a tool name belongs to the glossary Mutate (`write`, `edit`) or
/// Execute (`bash`) classes: the turn mutated state, so capture a checkpoint
/// before it. Read-only tools (read/grep/git_diff/list_directory/glob et al.)
/// never trigger a snapshot.
pub fn tool_needs_snapshot(tool_name: &str) -> bool {
    matches!(tool_name, "write" | "edit" | "bash")
}

/// True when any tool name in the turn belongs to Mutate/Execute (see
/// [`tool_needs_snapshot`]): the turn mutated state, so capture a checkpoint
/// before the mutate tool ran. A read-only turn needs no snapshot.
pub fn turn_needs_snapshot<'a, I>(tools: I) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    tools.into_iter().any(tool_needs_snapshot)
}

/// Build the PRE-turn checkpoint when the turn's tools touched state.
/// Returns `None` for read-only turns (no snapshot). The caller supplies the
/// pre-turn `git diff` and `messages_json` strings plus the current turn tag;
/// [`Context::push_checkpoint`] stores it bounded.
pub fn maybe_checkpoint<'a, I>(
    git_diff: &str,
    messages_json: &str,
    turn_idx: usize,
    tools: I,
) -> Option<Checkpoint>
where
    I: IntoIterator<Item = &'a str>,
{
    match turn_needs_snapshot(tools) {
        true => Some(Checkpoint {
            git_diff: git_diff.to_string(),
            messages_json: messages_json.to_string(),
            turn_idx,
        }),
        false => None,
    }
}

/// Caller-owned conversation history with tool-output truncation and
/// message-count compaction. In-memory only: dies with the process.
///
/// Budget: tool result texts are cut to [`Context::TOOL_BUDGET`] chars on
/// history entry. When the message count exceeds [`Context::TRIGGER`], the
/// oldest messages are summarized by the same model into a single user
/// message at index 0 and only [`Context::KEEP`] newest are retained.
pub struct Context {
    messages: Vec<Message>,
    checkpoints: std::collections::VecDeque<Checkpoint>,
    turn: usize,
}

impl Context {
    pub const TOOL_BUDGET: usize = 2000;
    pub const TRIGGER: usize = 100;
    pub const KEEP: usize = 40;

    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            checkpoints: std::collections::VecDeque::new(),
            turn: 0,
        }
    }

    /// Zero-based count of completed turns. Snapshot `turn_idx` tags each
    /// checkpoint so `/rewind n` can address `turn - n`.
    /// Test-only: the rewind/undo paths read `turn` inside this module.
    #[cfg(test)]
    pub fn turn_idx(&self) -> usize {
        self.turn
    }

    /// Advance once per completed `chat()` turn, success or failure.
    pub fn advance_turn(&mut self) {
        self.turn = self.turn.saturating_add(1);
    }

    /// The `chat()` wiring in one call: decide from the turn's tools whether
    /// the PRE-turn state (`git_diff` + pre-captured `messages_json`, both
    /// captured before the LLM ran) deserves a checkpoint, push it bounded,
    /// and always advance the turn counter. A read-only turn stores nothing.
    pub fn record_turn<'a, I>(&mut self, git_diff: &str, messages_json: &str, tools: I)
    where
        I: IntoIterator<Item = &'a str>,
    {
        if let Some(cp) = maybe_checkpoint(git_diff, messages_json, self.turn, tools) {
            self.push_checkpoint(cp);
        }
        self.advance_turn();
    }

    /// `/undo`: restore the PRE-turn state from the newest checkpoint and
    /// consume it (a second undo walks further back). Returns the consumed
    /// checkpoint so the caller can reverse-apply `git_diff` to the files.
    pub fn undo_last(&mut self) -> Result<Checkpoint, String> {
        let cp = self
            .checkpoints
            .pop_back()
            .ok_or_else(|| "no checkpoints to undo".to_string())?;
        self.restore_from(&cp)?;
        Ok(cp)
    }

    /// `/rewind n`: restore the state from `n` turns ago without consuming
    /// checkpoints (re-rewinding works). Picks the newest checkpoint at or
    /// before `turn - n`; clamps to the oldest snapshot when `n` reaches
    /// past the log.
    pub fn rewind_n(&mut self, n: usize) -> Result<Checkpoint, String> {
        let cp = self.peek_rewind(n)?;
        self.restore_from(&cp)?;
        Ok(cp)
    }

    /// The checkpoint [`Self::rewind_n`] would restore, without applying it.
    /// Lets the caller restore FILES from the exact target first, so a git
    /// failure leaves history untouched.
    pub fn peek_rewind(&self, n: usize) -> Result<Checkpoint, String> {
        if self.checkpoints.is_empty() {
            return Err("no checkpoints to rewind".to_string());
        }
        let target = self.turn.saturating_sub(n);
        Ok(self
            .checkpoints
            .iter()
            .filter(|c| c.turn_idx <= target)
            .max_by_key(|c| c.turn_idx)
            .or_else(|| self.checkpoints.front())
            .expect("checked non-empty")
            .clone())
    }

    /// Replace history with the checkpoint's `messages_json` and roll the
    /// turn counter back to the checkpointed turn.
    fn restore_from(&mut self, cp: &Checkpoint) -> Result<(), String> {
        let msgs: Vec<Message> = serde_json::from_str(&cp.messages_json)
            .map_err(|e| format!("checkpoint history unreadable: {e}"))?;
        self.messages = msgs;
        self.turn = cp.turn_idx;
        Ok(())
    }

    /// Oldest-first snapshot log for `/undo` and `/rewind`.
    pub fn checkpoints(&self) -> &std::collections::VecDeque<Checkpoint> {
        &self.checkpoints
    }

    /// Record one snapshot, dropping the oldest past [`MAX_CHECKPOINTS`].
    pub fn push_checkpoint(&mut self, cp: Checkpoint) {
        self.checkpoints.push_back(cp);
        Checkpoint::prune_checkpoints(&mut self.checkpoints, MAX_CHECKPOINTS);
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }

    // Session-restore helper, kept for upcoming resume wiring.
    #[allow(dead_code)]
    pub fn set_messages(&mut self, msgs: Vec<Message>) {
        self.messages = msgs;
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(&self.messages).map_err(|e| e.to_string())
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str::<Vec<Message>>(s)
            .map_err(|e| e.to_string())
            .map(|messages| Self {
                messages,
                checkpoints: std::collections::VecDeque::new(),
                turn: 0,
            })
    }

    #[cfg(test)]
    fn messages(&self) -> &Vec<Message> {
        &self.messages
    }

    /// Truncate `ToolResult` text items in `messages[from..]` to TOOL_BUDGET
    /// chars with a `[truncated N chars]` note. Call with the history length
    /// from before the latest turn so only newly committed messages are cut.
    pub fn truncate_new(&mut self, from: usize) {
        for msg in self.messages.iter_mut().skip(from) {
            if let Message::User { content } = msg {
                for item in content.iter_mut() {
                    if let UserContent::ToolResult(result) = item {
                        for c in result.content.iter_mut() {
                            if let ToolResultContent::Text(t) = c
                                && t.text.len() > Self::TOOL_BUDGET
                            {
                                let dropped = t.text.len() - Self::TOOL_BUDGET;
                                t.text.truncate(Self::TOOL_BUDGET);
                                t.text.push_str(&format!(" [truncated {dropped} chars]"));
                            }
                        }
                    }
                }
            }
        }
    }

    /// If over TRIGGER, split off the oldest messages for summarization.
    /// Returns them (caller summarizes, then calls [`Context::apply_summary`]).
    /// Keeps the newest KEEP verbatim. Never compacts twice without new turns:
    /// after compaction `len <= KEEP + 1`, well under TRIGGER.
    pub fn take_for_compaction(&mut self) -> Option<Vec<Message>> {
        if self.messages.len() <= Self::TRIGGER {
            return None;
        }
        let split_at = self.messages.len() - Self::KEEP;
        Some(self.messages.drain(..split_at).collect())
    }

    /// Replace drained messages with a single summary at index 0.
    pub fn apply_summary(&mut self, summary: String) {
        let entry = Message::user(format!("{SUMMARY_PREFIX} {summary}"));
        self.messages.insert(0, entry);
    }

    /// Local usage snapshot for `/context`: message count, estimated chars
    /// across user/assistant text (tool results included), the compaction
    /// trigger + keep window, and whether a summary is present.
    pub fn stats(&self) -> ContextStats {
        let mut chars = 0;
        let mut summarized = false;
        for m in &self.messages {
            match m {
                Message::System { content } => chars += content.chars().count(),
                Message::User { content } => {
                    for c in content {
                        match c {
                            UserContent::Text(t) => {
                                if t.text.starts_with(SUMMARY_PREFIX) {
                                    summarized = true;
                                }
                                chars += t.text.chars().count();
                            }
                            UserContent::ToolResult(r) => {
                                for part in &r.content {
                                    if let Some(text) = part.as_text() {
                                        chars += text.chars().count();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Message::Assistant { content, .. } => {
                    for c in content {
                        match c {
                            AssistantContent::Text(t) => chars += t.text.chars().count(),
                            AssistantContent::ToolCall(tc) => {
                                chars += tc.function.name.chars().count();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        ContextStats {
            messages: self.messages.len(),
            chars,
            trigger: Self::TRIGGER,
            keep: Self::KEEP,
            summarized,
        }
    }

    /// Render drained messages as compact `role: text` lines for the summarizer.
    pub fn render_for_summary(msgs: &[Message]) -> String {
        msgs.iter()
            .map(|m| match m {
                Message::System { content } => format!("system: {content}"),
                Message::User { content } => {
                    let parts: Vec<String> = content
                        .iter()
                        .map(|c| match c {
                            UserContent::Text(t) => t.text.chars().take(500).collect(),
                            UserContent::ToolResult(r) => {
                                let texts: Vec<&str> =
                                    r.content.iter().filter_map(|c| c.as_text()).collect();
                                let joined = texts.join(" | ");
                                let cut: String = joined.chars().take(500).collect();
                                format!("[{} result] {cut}", r.name)
                            }
                            _ => "[non-text content]".to_string(),
                        })
                        .collect();
                    format!("user: {}", parts.join(" | "))
                }
                Message::Assistant { content, .. } => {
                    let parts: Vec<String> = content
                        .iter()
                        .map(|c| match c {
                            AssistantContent::Text(t) => t.text.chars().take(500).collect(),
                            AssistantContent::ToolCall(tc) => {
                                format!("[tool call: {}]", tc.function.name)
                            }
                            _ => "[non-text content]".to_string(),
                        })
                        .collect();
                    format!("assistant: {}", parts.join(" | "))
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::ToolResult;

    fn tool_msg(text: &str) -> Message {
        Message::User {
            content: vec![UserContent::ToolResult(ToolResult {
                call: "test-call".to_string().try_into().unwrap(),
                provider: None,
                name: "read".to_string(),
                content: vec![ToolResultContent::text(text)],
            })],
        }
    }

    #[test]
    fn stats_reports_message_count_chars_and_summary_presence() {
        let mut ctx = Context::new();
        assert_eq!(
            ctx.stats(),
            ContextStats {
                messages: 0,
                chars: 0,
                trigger: Context::TRIGGER,
                keep: Context::KEEP,
                summarized: false,
            }
        );
        ctx.messages_mut().push(Message::user("hello world"));
        ctx.messages_mut().push(tool_msg("abc"));
        let stats = ctx.stats();
        assert_eq!(stats.messages, 2);
        assert_eq!(stats.chars, "hello world".len() + "abc".len());
        assert_eq!(stats.trigger, Context::TRIGGER);
        assert_eq!(stats.keep, Context::KEEP);
        assert!(!stats.summarized);
        ctx.apply_summary("did stuff".to_string());
        let stats = ctx.stats();
        assert_eq!(stats.messages, 3);
        assert!(stats.summarized);
    }

    #[test]
    fn truncates_long_tool_results_with_note() {
        let mut ctx = Context::new();
        ctx.messages_mut().push(tool_msg(&"x".repeat(2500)));
        ctx.truncate_new(0);
        let Message::User { content } = &ctx.messages()[0] else {
            panic!("expected user msg");
        };
        let UserContent::ToolResult(r) = &content[0] else {
            panic!("expected tool result");
        };
        let text = r.content[0].as_text().unwrap();
        assert!(text.len() <= Context::TOOL_BUDGET + 40);
        assert!(text.contains("[truncated 500 chars]"));
    }

    #[test]
    fn leaves_short_results_alone() {
        let mut ctx = Context::new();
        ctx.messages_mut().push(tool_msg("short"));
        ctx.truncate_new(0);
        let Message::User { content } = &ctx.messages()[0] else {
            panic!("expected user msg");
        };
        let UserContent::ToolResult(r) = &content[0] else {
            panic!("expected tool result");
        };
        assert_eq!(r.content[0].as_text().unwrap(), "short");
    }

    #[test]
    fn only_truncates_from_offset() {
        let mut ctx = Context::new();
        ctx.messages_mut().push(tool_msg(&"y".repeat(2500)));
        ctx.messages_mut().push(Message::user("hello"));
        ctx.truncate_new(1); // skip the long tool message
        let Message::User { content } = &ctx.messages()[0] else {
            panic!("expected user msg");
        };
        let UserContent::ToolResult(r) = &content[0] else {
            panic!("expected tool result");
        };
        assert_eq!(r.content[0].as_text().unwrap().len(), 2500);
    }

    #[test]
    fn compaction_keeps_newest_and_applies_summary() {
        let mut ctx = Context::new();
        for i in 0..Context::TRIGGER + 10 {
            ctx.messages_mut().push(Message::user(format!("msg {i}")));
        }
        let drained = ctx.take_for_compaction().expect("should compact");
        assert_eq!(drained.len(), Context::TRIGGER + 10 - Context::KEEP);
        assert_eq!(ctx.len(), Context::KEEP);
        // newest retained
        let Message::User { content } = &ctx.messages()[Context::KEEP - 1] else {
            panic!("expected user msg");
        };
        assert!(format!("{content:?}").contains("msg 109"));
        ctx.apply_summary("did stuff".to_string());
        assert_eq!(ctx.len(), Context::KEEP + 1);
        let rendered = Context::render_for_summary(&drained);
        assert!(rendered.contains("msg 0"));
    }

    #[test]
    fn no_compaction_under_trigger() {
        let mut ctx = Context::new();
        ctx.messages_mut().push(Message::user("hi"));
        assert!(ctx.take_for_compaction().is_none());
    }

    #[test]
    fn checkpoint_struct_creates_with_all_fields() {
        let cp = Checkpoint {
            git_diff: "diff content".to_string(),
            messages_json: r#"[{"role":"user","content":"test"}]"#.to_string(),
            turn_idx: 5,
        };
        assert_eq!(cp.git_diff, "diff content");
        assert_eq!(cp.messages_json, r#"[{"role":"user","content":"test"}]"#);
        assert_eq!(cp.turn_idx, 5);
    }

    #[test]
    fn new_checkpoint_creates_snapshot_from_context() {
        let mut ctx = Context::new();
        ctx.messages_mut().push(Message::user("hello world"));
        let cp = Checkpoint::new_checkpoint(&ctx, 3, "git diff output");
        assert_eq!(cp.turn_idx, 3);
        assert_eq!(cp.git_diff, "git diff output");
        // messages_json should be the serialized messages
        let parsed: Vec<Message> = serde_json::from_str(&cp.messages_json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], Message::user("hello world"));
    }

    #[test]
    fn prune_checkpoints_drops_oldest_when_over_limit() {
        // Create checkpoints manually to test pruning
        use std::collections::VecDeque;
        const MAX: usize = 3;
        let mut checkpoints: VecDeque<Checkpoint> = VecDeque::new();
        // Add 4 checkpoints (over the limit)
        for i in 0..4 {
            checkpoints.push_back(Checkpoint {
                git_diff: format!("diff {}", i),
                messages_json: format!("[msg{}]", i),
                turn_idx: i,
            });
        }
        // Prune should drop the oldest (first one)
        let pruned = Checkpoint::prune_checkpoints(&mut checkpoints, MAX);
        assert_eq!(checkpoints.len(), MAX);
        // The oldest (turn_idx 0) should be gone
        assert!(!pruned.iter().any(|c| c.turn_idx == 0));
        // The newest (turn_idx 3) should remain
        assert!(pruned.iter().any(|c| c.turn_idx == 3));
    }

    #[test]
    fn context_holds_bounded_checkpoint_log() {
        let mut ctx = Context::new();
        assert_eq!(ctx.checkpoints().len(), 0);
        for i in 0..(MAX_CHECKPOINTS + 5) {
            ctx.push_checkpoint(Checkpoint {
                git_diff: format!("diff {i}"),
                messages_json: format!("[msg{i}]"),
                turn_idx: i,
            });
        }
        assert_eq!(ctx.checkpoints().len(), MAX_CHECKPOINTS);
        // Oldest dropped: turn 0..5 gone, newest retained.
        assert_eq!(ctx.checkpoints()[0].turn_idx, 5);
        assert_eq!(
            ctx.checkpoints().back().unwrap().turn_idx,
            MAX_CHECKPOINTS + 4
        );
    }

    #[test]
    fn snapshot_worthy_tools_match_glossary_mutate_and_execute() {
        // Glossary: Mutate = write/edit, Execute = bash. Read-only tools
        // (read, grep, git_diff, list_directory) need no snapshot.
        for tool in ["write", "edit", "bash"] {
            assert!(
                tool_needs_snapshot(tool),
                "{tool} should trigger a snapshot"
            );
        }
        for tool in [
            "read",
            "grep",
            "git_diff",
            "git_status",
            "list_directory",
            "glob",
        ] {
            assert!(
                !tool_needs_snapshot(tool),
                "{tool} should not trigger a snapshot"
            );
        }
    }

    #[test]
    fn turn_needs_snapshot_when_any_mutate_or_execute_tool_ran() {
        // One tracer: a turn with only reads needs no snapshot; a turn
        // where any Mutate/Execute tool ran does.
        assert!(!turn_needs_snapshot(["read", "grep"]));
        assert!(!turn_needs_snapshot([] as [&str; 0]));
        assert!(turn_needs_snapshot(["read", "edit"]));
        assert!(turn_needs_snapshot(["bash"]));
    }

    #[test]
    fn maybe_checkpoint_builds_only_when_mutate_tool_ran() {
        // Read-only turn: no checkpoint. Mutate turn: checkpoint carries
        // the PRE-turn diff + messages + turn tag.
        assert!(maybe_checkpoint("d", "m", 7, ["read", "grep"]).is_none());
        assert!(maybe_checkpoint("d", "m", 7, [] as [&str; 0]).is_none());
        let cp = maybe_checkpoint("d", "m", 7, ["read", "edit"]).expect("edit turn snapshots");
        assert_eq!(cp.git_diff, "d");
        assert_eq!(cp.messages_json, "m");
        assert_eq!(cp.turn_idx, 7);
    }

    #[test]
    fn turn_counter_advances_once_per_completed_turn() {
        let mut ctx = Context::new();
        assert_eq!(ctx.turn_idx(), 0);
        ctx.advance_turn();
        assert_eq!(ctx.turn_idx(), 1);
        ctx.advance_turn();
        assert_eq!(ctx.turn_idx(), 2);
    }

    #[test]
    fn record_turn_wires_capture_decide_push_advance() {
        // Mirrors the chat() wiring: pre-turn state is captured up front,
        // the decision comes from the turn's tools, the counter always advances.
        let mut ctx = Context::new();
        ctx.messages_mut().push(Message::user("hello"));
        let pre_json = ctx.to_json().unwrap();

        // Read-only turn: no checkpoint, but the turn still advances.
        ctx.record_turn("diff-before", &pre_json, ["read", "grep"]);
        assert_eq!(ctx.checkpoints().len(), 0);
        assert_eq!(ctx.turn_idx(), 1);

        // Mutate turn: checkpoint stores the PRE-turn diff + messages.
        ctx.messages_mut().push(Message::user("edit landed"));
        let pre_json2 = ctx.to_json().unwrap();
        ctx.messages_mut()
            .push(Message::user("assistant saw the edit"));
        ctx.record_turn("diff-before-edit", &pre_json2, ["read", "edit"]);
        assert_eq!(ctx.checkpoints().len(), 1);
        let cp = &ctx.checkpoints()[0];
        assert_eq!(cp.git_diff, "diff-before-edit");
        assert_eq!(cp.turn_idx, 1);
        // messages_json is the PRE-edit history: 2 messages, not 3.
        let parsed: Vec<Message> = serde_json::from_str(&cp.messages_json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(ctx.turn_idx(), 2);
    }

    #[test]
    fn undo_last_restores_pre_turn_history_and_consumes_checkpoint() {
        let mut ctx = Context::new();
        ctx.messages_mut().push(Message::user("before edit"));
        let pre = ctx.to_json().unwrap();
        ctx.messages_mut().push(Message::user("edit landed"));
        ctx.record_turn("diff-1", &pre, ["edit"]);
        assert_eq!(ctx.len(), 2);

        let cp = ctx.undo_last().expect("undo works");
        // History rolled back to the PRE-edit snapshot (1 message).
        assert_eq!(ctx.len(), 1);
        // Turn counter rolled back to the checkpointed turn.
        assert_eq!(ctx.turn_idx(), cp.turn_idx);
        // The checkpoint is consumed: a second undo finds nothing.
        assert_eq!(ctx.checkpoints().len(), 0);
        assert!(ctx.undo_last().is_err());
    }

    #[test]
    fn undo_last_errors_when_no_checkpoints() {
        let mut ctx = Context::new();
        let err = ctx.undo_last().unwrap_err();
        assert!(err.contains("no checkpoints"), "got: {err}");
    }

    #[test]
    fn rewind_n_restores_that_turn_and_clamps_to_oldest() {
        let mut ctx = Context::new();
        // Two mutate turns: checkpoints at turn 0 and turn 1.
        ctx.messages_mut().push(Message::user("t0"));
        let p0 = ctx.to_json().unwrap();
        ctx.record_turn("d0", &p0, ["bash"]); // cp turn_idx 0
        ctx.messages_mut().push(Message::user("t1"));
        let p1 = ctx.to_json().unwrap();
        ctx.record_turn("d1", &p1, ["edit"]); // cp turn_idx 1
        ctx.messages_mut().push(Message::user("t2"));
        assert_eq!(ctx.turn_idx(), 2);
        assert_eq!(ctx.checkpoints().len(), 2);

        // rewind 1 from turn 2 -> turn 1 checkpoint (1 message at capture).
        let cp = ctx.rewind_n(1).expect("rewind 1 works");
        assert_eq!(cp.turn_idx, 1);
        assert_eq!(ctx.turn_idx(), 1);
        assert_eq!(ctx.len(), 2); // t0 + t1 restored

        // rewind 99 clamps to the oldest checkpoint (turn 0).
        let cp = ctx.rewind_n(99).expect("clamped rewind works");
        assert_eq!(cp.turn_idx, 0);
        assert_eq!(ctx.turn_idx(), 0);
        assert_eq!(ctx.len(), 1); // only t0

        // Rewind does not consume checkpoints: the log survives.
        assert_eq!(ctx.checkpoints().len(), 2);
    }

    #[test]
    fn rewind_n_errors_when_no_checkpoints() {
        let mut ctx = Context::new();
        let err = ctx.rewind_n(1).unwrap_err();
        assert!(err.contains("no checkpoints"), "got: {err}");
    }
}
