# ADR-0016: Parallel fan-out + background tasks (`task` v2, `/tasks`)

Status: Accepted
Date: 2026-10-08
Grilled: P0-#6 slices 03–04 (parallel approvals, background lifecycle).

## Decision

- `task` gains two shapes under one tool, exactly-one enforced in
  `call()` with model-visible errors (JSON Schema cannot express XOR):
  single (`description` + `prompt`, v1 behavior: raw summary, child
  errors propagate) and fan-out (`tasks[]` of `{description, prompt}`).
- Fan-out runs bounded-parallel on a scoped `JoinSet` inside the tool
  call: `FANOUT_CONCURRENCY=4` semaphore, `FANOUT_MAX_TASKS=8` hard cap
  (over-cap rejected so the model splits the batch; excess under the cap
  queues). Merge is `## <description>` blocks in input order with
  `## <label> (error)` per-child blocks — partial merge, one child
  failing never fails the batch. The set is never detached: dropping the
  tool future aborts every child, so Esc keeps the serial abort story
  with zero TUI work.
- Child gates are labeled (`PermissionHook::with_task_label`): same
  approval channel (one FIFO queue, no interleaving change), but the
  modal shows `tool [label]` + `[subagent label]` reason prefix so
  parallel Confirms attribute. Deny floor unchanged.
- `background:true` detaches the whole call (single or fan-out) via
  `tokio::spawn` into a session `BackgroundStore`
  (`Arc`, same shape as `mode`); `call()` returns `background task-N
  started (...)` immediately. The worker finishes with the merged result
  (single: raw summary or `error: …`; fan-out: same `##` merge).
  Summaries truncate to `FULL_OUTPUT_CAP`.
- Store is session-memory only, never persisted; `/new` clears it
  alongside context + todos. Headless `-p` disables the store:
  `background:true` degrades to inline (same result, no polling).
- Result return is TUI-inject-on-next-submit: `submit()` drains
  `take_background_completed()` (exactly-once) and prepends
  `background <id> finished (<labels>):\n<summary>` blocks to the model
  prompt. The human polls via `/tasks` (list) and `/tasks <id>`
  (detail, summary replay on demand).
- `task_preview` forks on `tasks[]` presence (`N subtasks: a, b, …`) with
  a `[background]` suffix; `task` stays `Allow` in every mode; depth-1
  denied-at-schema unchanged (children never receive `task`).

## Rationale

- One tool, not two: the model already knows `task`; a `tasks[]` array
  is a smaller conceptual step than a `task_fanout` sibling, and the
  compat cost is one XOR branch plus a parse test.
- Scoped `JoinSet` over an abort registry: the serial invariant
  (drop-the-future-cancels-the-child) extends to N children for free;
  an explicit registry couples fan-out to background machinery early.
- Label-prefix over per-child queue UI: preserves the serial-first
  approval story (ADR-0015) — one queue, one approver — with attribution
  as a string, not a state machine.
- Inject-on-submit over model polling: no new tool in the preamble, no
  spin-poll loops; the model stays dumb, the human stays informed via
  `/tasks`.
- Session-memory over persistence: background results are turn-scoped
  context, not session history; resume keeps history, not live workers.

## Consequences

- Parent tool count stays 13, child 12. `/model` / `/effort` /
  `/skills reload` rebuilds thread the shared `BackgroundStore` through
  every fresh `SerialTaskRunner` so future children follow switches and
  the store survives rebuilds.
- `TaskTool::with_runner` is gone (store required);
  `TaskArgs.description/prompt` are `Option` (validation moved to
  `call()`); `task_args_reject_missing_prompt` now asserts at call, not
  parse.
- `/tasks` joins the slash registry (16 commands); `/new` clears the
  store; `/help` documents fan-out + background.

## Tests

- Tool seam (no LLM): input-order merge, partial merge with inline
  child errors, over-cap/empty/both-shapes rejection, 5-child queue
  completion, preview fan-out + background suffix, background detach →
  store completion → exactly-once drain, disabled-store inline,
  compat (old shape still returns raw summary).
- Policy seam: labeled child shares the parent channel; Confirm carries
  `[label]` tool name + `[subagent label]` reason; preview unchanged.
- Production seams (no LLM): child `tool_definitions` still excludes
  `task`; `build_serial_child` labels the gate; history untouched.
- TUI: `/tasks` list/detail/unknown-id dispatch (no turn), `/help`
  fan-out strings, 16-command wrap math.
