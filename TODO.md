# rem — TODO: parity with Claude Code / Codex CLI / Pi

Source: gap analysis of `rem` (dev @ d819188) vs Claude Code, Codex CLI, Pi agent.
Last updated: 2026-10-06

Legend: `[ ]` todo, `[/]` in-progress branch, `[x]` done.

## P0 — not competitive without these

### 1. Fix broken tree + ship baseline
- [x] Resolve merge conflicts in `src/agent.rs`, `src/main.rs`, `src/tui.rs` (`feat/skills` markers)
- [x] `cargo build`, `cargo test`, `cargo fmt`, `cargo clippy -D warnings` green
- [x] Merge `feat/skills` cleanly into `dev`
- [x] Add license (MIT/Apache-2.0), flesh out README + `--help`

### 2. Headless / automation interfaces (grilled 2026-10-06: `-p` only, no `exec`)
- [x] `rem -p "<prompt>"` print mode (pipe final response to stdout; exit 0/1/2)
- [x] ~~`rem exec`~~ dropped: one canonical `-p` path (bare positionals hint at `-p`)
- [x] `--output text|json|jsonl` event stream (tools + result/error; text default)
- [x] Stdin pipe support (`echo ... | rem -p`; stdin first + `-p` second, either alone)
- [x] `--mode <name>` single flag only (no `--yolo`/`--plan` shorthands; default `manual`)
- [x] `--resume <id>` works headless; deny-by-default + `--allow-all` (Confirm→Approve, Deny floor holds)

### 3. MCP client (landed 2026-10-06, tickets 01-06 in `.scratch/mcp-client/issues/`)
- [x] Land MCP client: stdio + streamable HTTP (legacy SSE out — rmcp 2.2 ships no SSE client dialect, per ADR-0013)
- [x] MCP servers from `~/.config/rem/mcp.json` (Claude `mcpServers` shape, `${VAR}`/`${VAR:-default}` expansion; supersedes `mcp.toml` wording)
- [x] Discovered MCP tools flow through `PermissionHook::classify` (explicit `mcp__*` Confirm arm; Yolo Allow, Plan Deny; High risk)
- [x] `/mcp` command: list servers/tools, on/off toggle, login/logout, connected/failed/needs-auth status
- [x] MCP tools appear in preamble index + `last_tool_events` tree; full OAuth 2.0 (0600 token store, localhost callback, 401 refresh-once → needs-auth)

### 4. `@file` / ref-files (`feat/ref-files` landed as `f5589b8`; `@dir/` + multi-ref + blocks verified, ranges + docs landed 2026-10-06)
- [x] Land `feat/ref-files`: `@path` expansion before `chat()`
- [x] Support `@dir/`, multiple refs per prompt (appearance order, `list_directory` parity, verified in tree)
- [x] Support `@file:line-range` (`START-END`, `START-` to EOF, single `LINE`; numbered lines; `END` clamps; invalid/range-on-dir blocks naming the ref; cap respected; popup preserves suffix)
- [x] Missing-file error surfaces in TUI, doesn't send turn (+ binary/outside-root/invalid-range blocks, same channel)
- [x] Docs + `/help` entry (`/help` footer, README section, glossary + ADR-0012 notes)

### 5. Providers + models (multi-provider: OpenAI-compatible + Anthropic native)
- [x] Multi-profile config (`[providers.*]`, env-var keys, no hardcoded single `[api]`)
- [x] `/model` switch mid-session + `/effort` (`low/medium/high`)
- [ ] Anthropic native, Bedrock/Vertex passthrough or OAuth + `/login`
- [x] Retry + rate-limit backoff + timeout errors surfaced per-turn
- [ ] Token meter + `/cost` + `/context` (used/total, compaction point)

### 6. Subagents + task decomposition (serial v1 landed 2026-10-07: ADR-0015; parallel + background landed 2026-10-08: ADR-0016)
- [x] `TodoWrite` tool (list, status, transcript UI)
- [x] `Task` tool: spawn subagent with scoped prompt/tools (serial queue v1: fresh context, shared approval channel, depth-1 denied-at-schema)
- [x] Parallel subagent fan-out (bounded) + result merge into parent turn
- [x] Background tasks with polling (`/tasks`)

### 7. Checkpoints / undo / rewind (no revert today)
- [x] Snapshot `git diff` + `messages_json` before each mutate turn
- [x] `/rewind [n]` restores files + history; `/undo` last mutate
- [x] Diff-accept flow: show patch, `[a]ccept / [r]eject` before `edit/write` lands
- [x] `apply_patch`-style fuzzy edit tool (replace exact-`oldText`-only `edit`)

## P1 — expected by users

- [ ] Live token streaming (today: tool completions only, no token stream)
- [ ] Plan artifact: `plan` mode writes plan file, approval-to-implement handoff
- [ ] Memory hierarchy: `AGENTS.md` walk-up + `~/.config/rem/AGENTS.md` + `@import` + `/memory`; hot-reload (no restart)
- [ ] Custom commands: `~/.config/rem/commands/*.md` + project `.agents/commands/*.md` (prompt templates)
- [ ] User hooks: pre/post tool-call hooks from config (command hooks a la Codex)
- [ ] Extensions API or WASM/plugin tools (or defer to MCP-only — decide)
- [ ] Images / multimodal: paste image, `read` PDFs/screenshots as content blocks
- [ ] Trust model: `project_trust` prompt before loading project `AGENTS.md`/skills/MCP
- [ ] Session branches: continue-from-earlier = new branch; `/branch`, history tree UI
- [ ] Session export/share (`/export`, JSONL dump); `list_all` picker across projects
- [ ] GitHub workflow: `/pr`, `gh` integration, worktree support
- [ ] Background `bash` (async run + `poll`, no 30s kill)
- [ ] Diagnostics loop: LSP / `cargo check` feedback after edits
- [ ] Wire `generate_title()` (today every session is `untitled` + 40-char fallback)
- [ ] `/skills` extras: `disable-model-invocation`, `user-invocable`, `allowed-tools`, `when_to_use`

## P2 — product maturity

- [ ] Distribution: `cargo install rem`, brew/npm, auto-update check, shell completions
- [ ] `/doctor`: config, provider ping, MCP servers, theme, db health
- [ ] Config hierarchy: global + project-local `.rem/config.toml` + env overrides + wizard
- [ ] Keybinding config (today hardcoded Esc/Ctrl-C/Ctrl-D/Tab)
- [ ] Observability: `--verbose` log file, persisted audit trail of approvals, cost/latency trailer (`Done: Xs · N tools · $c`)
- [ ] Hardening: seatbelt/Docker/container mode; secrets redaction on `read`; symlink-safe `inside_project`
- [ ] Docs: `docs/cli.md`, `docs/permissions.md`, `docs/sessions.md`, `docs/skills.md` parity with Pi docs set

## Tech debt / correctness

- [ ] `edit` uniqueness check is `O(n·m)` string scan — fine, but error shows only 120 chars; include line numbers
- [ ] `split_segments` shell parser: document as heuristic, add fuzz tests for `$(...)`, quotes, `#` comments
- [ ] `Context::TOOL_BUDGET=2000` chars vs bytes mismatch (`truncate` on bytes, note on `len()`); unify on chars
- [ ] SQLite: WAL mode, index on `project_root, updated_at`; prune old sessions (`/prune`)
- [ ] TUI (5251 lines): split `tui.rs` into `tui/{app,events,render,commands}.rs`
- [ ] Glossary promises `permission ...` transcript blocks — implement or drop the claim
- [ ] CI: runagate `feat/mcp` + `feat/ref-files` merges through `dev` test + clippy gate
