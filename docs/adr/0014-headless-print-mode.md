# ADR-0014: Headless print mode (`rem -p`)

Status: Accepted
Date: 2026-10-06
Grilled: P0-#2 interview (7 decisions, all applied).

## Decision

- Single canonical flag: `-p` / `--print` (no `exec` subcommand). Trailing
  bare args join into the prompt; bare positionals without `-p` are a usage
  error hinting at `-p`. `--help` / `--version` win over everything.
- Prompt = piped stdin (context) + `-p` (instruction); stdin first, either
  alone allowed. TTY stdin reads as empty (never blocks). Empty both sides
  is exit 2 and touches neither config nor SQLite.
- Single `--mode <name>` only (`plan|manual|auto|edit|yolo`, default
  `manual`); no `--yolo` / `--plan` shorthands. Applied to the agent's
  `mode_handle` before the turn so `classify_mode` sees it.
- Deny-by-default: no approver attached — the `ApprovalRx` is dropped so
  `Confirm` fails closed to in-run skip (existing `PermissionHook` path:
  `send` fails -> `Skip`, model replans). `--allow-all` spawns a background
  auto-approver (`Confirm` -> `Approve`, empty comment). `Deny` verdicts
  (annihilators) never reach the channel, so the safety floor holds in
  every mode, including `yolo` + `--allow-all`.
- Output: `text` default (tool `● name preview — summary` lines to stderr,
  final reply to stdout), `--output json` (one pretty doc: tools + terminal
  result/error object), `--output jsonl` (one object per line). Provider
  errors also echo to stderr so pipes fail visibly.
- Sessions persist on success only (TUI parity: `save_messages` + `touch` +
  40-char title fallback from the unexpanded prompt). Resume hint goes to
  stderr so stdout pipes cleanly. `--resume <id>` switches to the stored
  `provider:model` tag (warn + keep history on unknown tags) and imports
  history before the turn. Unknown resume ids exit 1.
- `@path` + `/skill:name` expansion reuses the TUI submit path verbatim
  (`expand_skill_invokes` first so `@refs` inside skill tasks resolve, then
  `expand_at_refs`); failures are fatal headless (exit 1) instead of a
  transcript notice.
- Exit codes: `0` turn ok, `1` turn/provider/save/config/session error,
  `2` CLI usage error. No TUI, no raw mode, no viewport teardown.

## Rationale

- One path (`-p`) beats two (`-p` + `exec`): same runner, fewer flags to
  document, and bare-positional hinting teaches the canonical form.
- Dropping the receiver reuses the already-tested fail-closed hook path
  instead of inventing a headless policy fork; `--allow-all` is a dumb
  approver, not a policy bypass, so `Deny` physically cannot be approved.
- stderr-for-diagnostics / stdout-for-payload keeps
  `rem -p ... | jq` and `rem -p ... > out.txt` scriptable, matching the
  `text`-default / `json`-opt-in grill decision.
- Persist-on-success mirrors the TUI (failed turns don't checkpoint
  half-history), and stderr resume hints preserve pipeability.

## Consequences

- `src/headless.rs` owns the runner (`PrintOpts`, `combine_prompt`,
  `read_stdin_piped`, JSON renderers, `run_print`); `src/main.rs` owns CLI
  parsing; `src/modes.rs` gains `Mode::from_name`; `src/tui.rs` exposes
  `expand_at_refs` / `expand_skill_invokes` as `pub(crate)`.
- README `Limits` no longer claims "no headless allow"; TODO P0-#2 marked
  done; glossary `Approver` covers the auto-approver.
- Live-verified: plan-mode read + `@ref` + stdin + resume chaining green;
  `Confirm` denied without `--allow-all`, approved with it; JSON/JSONL parse
  clean; usage errors exit 2 without config/db side effects.
