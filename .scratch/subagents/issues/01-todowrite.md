# 01: TodoWrite list tracking with transcript UI

**What to build:** The Principal gains a `todowrite` tool that owns the session's task list (content + status per item) on shared agent state, readable from the transcript UI. No persistence, no priorities, no IDs beyond list order.

**Blocked by:** None (can start immediately).

**Status:** done

- [x] `todowrite` tool args are an items array; each item carries `content` (string) + `status` (`pending` / `in_progress` / `completed`); unknown fields such as `priority` are accepted and ignored so Claude-authored prompts don't break
- [x] List state lives on an `Arc`-shared list owned by the agent (same `Arc<Mutex>` shape as `mode`), mutated by the tool and readable via a new defaulted `AgentLoop` listing method; never written into caller-owned history or `sessions.db` for v1
- [x] `classify()` returns `Allow` for `todowrite` in every permission mode including `plan` (local UI state only; no files, shell, or network by itself)
- [x] Transcript renders the list through the existing tool-row path (label + summary + elided output); `/new` resets the list alongside context
- [x] Covered at the Tool + Policy seam without LLM or TUI: direct `Tool::call` round-trip (content+status persistence, unknown-priority ignored, bad-status rejected), `classify()` `Allow` in all modes, list reset on new session
