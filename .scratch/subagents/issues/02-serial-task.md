# 02: Serial Task subagent with scoped prompt and gated tools

**What to build:** The Principal gains a `task` tool that runs one scoped subagent to completion inside the current Run and returns only its final summary. The child gets a fresh Context plus the Task prompt under the identical preamble, the same 11 file/shell/git/search/web tools plus `todowrite` but never `task` (depth-1, no recursion), and its Confirm verdicts park on the same Gate approval channel the Approver already answers. Parent transcript shows a single `task` tool row; inner steps are stored bounded, never streamed.

**Blocked by:** 01: TodoWrite list tracking (`todowrite` must exist so the child toolset can include it).

**Status:** done

- [x] `task` tool args are `description` (short transcript label) + `prompt` (full scoped task); fixed budget inherited from the parent leg (same max turns, same 30s bash); no `max_turns` / timeout knobs for v1
- [x] Child runs a fresh inner agent built from the same staged config + live effort plus a fresh caller-owned Context and fresh recorder; parent history is never cloned or summarized into the child
- [x] Child preamble matches the parent (static instructions + project instructions + skills index) plus a delegation note naming the assignment and demanding a final summary; unknown `priority`-style extras are not part of this tool
- [x] `task` itself is absent from the child tool list so a subagent cannot spawn (depth-1 only; denied-at-schema, not merely policy)
- [x] `classify()` returns `Allow` for `task` in every permission mode including `plan`; every tool call the child makes is classified by the same Policy through the shared Gate instance (annihilator Deny floor holds; Confirm parks serialized on the one approval channel)
- [x] Parent ToolEvent for `task` carries the description as preview and the child summary as output bounded by the existing full-output cap; transcript renders it as one tool row with the existing head/tail elision, no live inner-step rows
- [x] Aborting the parent turn cancels the in-flight child future; headless keeps deny-by-default with `--allow-all` auto-approving child Confirms while Deny still blocks
- [x] Covered at the Tool + Policy seam without LLM or TUI: arg parsing (description+prompt, missing prompt rejected), child toolset excludes `task`, `classify()` `Allow` in all modes, fresh-Context isolation (parent history untouched), serialized approvals through the shared Gate
