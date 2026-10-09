# ADR-0015: Serial subagents v1 (`todowrite` + `task`)

Status: Accepted
Date: 2026-10-07
Grilled: P0-#6 slices 01–02 (Tool + Policy seams, no LLM/TUI tests).

## Decision

- `todowrite` owns the session task list on agent-shared state
  (`Arc<Mutex<Vec<TodoItem>>>`, same shape as `mode`): `content` + `status`
  (`pending` / `in_progress` / `completed`) per item, full-list replace per
  call. Unknown fields (Claude-style `priority`) accepted and ignored.
  `Allow` in every mode including `plan` (local UI state only). Never
  persisted to `sessions.db`; `/new` clears it alongside context.
- `task` runs one scoped subagent serially inside the current run and
  returns only its final summary. Args: `description` (transcript label) +
  `prompt` (full task). Fixed budget inherited from the parent leg (same
  100 max turns, same 30s bash); no `max_turns` / timeout knobs for v1.
- Child gets a fresh inner agent from the same staged config + live effort,
  a fresh caller-owned `Context`, fresh recorder, and fresh todo list.
  Parent history is never cloned or summarized into the child. Preamble is
  structurally identical (shared `base_preamble` + project instructions +
  skills index) plus a delegation note naming the assignment and demanding
  a final summary.
- Depth-1 denied-at-schema: `task` is registered on the parent only, never
  on the child. Policy needs no recursion rule.
- `task` itself is `Allow` in every mode including `plan`; every tool call
  the child makes is classified by the same Policy through the shared
  `PermissionHook` clone (same approval channel, so Confirms serialize on
  the one queue; annihilator Deny floor holds). Headless keeps
  deny-by-default with `--allow-all` auto-approving child Confirms.
- Parent transcript shows a single `task` row (description preview, summary
  output bounded by `FULL_OUTPUT_CAP`); inner steps are stored bounded in
  the child's recorder, never streamed. Aborting the parent turn cancels
  the in-flight child future (awaited inline, never `tokio::spawn`).

## Rationale

- Serial-first keeps the approval story trivial: one queue, one approver,
  no interleaving. Parallel fan-out would need per-child attribution in
  the modal — deferred, not rejected.
- Fresh context per child avoids history-bloat and cross-task leakage; the
  delegation note + summary is the entire parent/child contract.
- Schema-level depth-1 (rather than a policy Deny on nested `task`) means
  the model never sees a `task` tool inside a child, so it can't plan
  around a refusal.

## Consequences

- Parent tool count 13, child 12. `/model` / `/effort` / `/skills reload`
  rebuilds construct a fresh `SerialTaskRunner` from the staged leg so
  future children follow switches.
- Follow-ups (deferred): bounded parallel fan-out + result merge,
  background tasks with polling (`/tasks`).

## Tests

- Tool seam (no LLM): arg parsing, blank rejection, fake-runner
  summary round-trip, child-failure surfacing, `task_preview`.
- Policy seam: `task` Allow in all 5 modes; `todowrite` Allow in all 5.
- Production seams (no LLM): child `tool_definitions` excludes `task`;
  child gate shares the parent approval channel (`same_channel`); child
  construction leaves parent history untouched.
- TUI: `/help` documents shapes + v1 limits (assertion test).
