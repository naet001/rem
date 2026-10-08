# rem

A terminal coding agent built as a personal learning project. Rust + Ratatui TUI and a Rig agent loop. Every tool call runs immediately — no permission gates, no approval prompts.

## Purpose

`rem` is a simple harness for learning how coding agents are put together. It is not production software: expect rough edges, and read the safety story below as a study, not a guarantee.

The project works through a few questions end to end:

- How do you build a full-screen TUI — panes, popups, themes, keymaps — in Rust with Ratatui?
- How do you run an agent loop (Rig) with tool calls, subagents, and session persistence?
- How do sessions, skills, and config persist without any of it becoming privileged or magic?

## Design notes

The answers, as implemented:

- **Tools run free** — every tool call executes immediately. No gates, no prompts, no modes.
- **Reusable knowledge** — Claude-compatible `SKILL.md` bundles with startup index + on-demand bodies (ADR-0013).
- **Decisions on record** — 19 ADRs in `docs/adr/`, `cargo test` green (minus 2 live web checks that need network).

## Quickstart

Requires Rust + an OpenAI-compatible endpoint and/or an Anthropic key.

```bash
mkdir -p ~/.config/rem
cp config.example.toml ~/.config/rem/config.toml
# edit ~/.config/rem/config.toml: set active_provider + [providers.*] entries;
# export OPENAI_API_KEY / ANTHROPIC_API_KEY to match your key = "env:..." refs
cargo run
```

Minimal `~/.config/rem/config.toml`:

```toml
active_provider = "openai"
effort = "medium"  # low / medium / high (unified, mapped per provider)

[providers.openai]
type = "openai-compatible"
base_url = "https://api.openai.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"
```

Add a `[providers.anthropic]` block (see `config.example.toml`) to use
Claude too. Keys use `env:VAR` indirection only — literals are rejected.
Switch live with `/model`, tune reasoning with `/effort`.

Talk, quit with `/quit`. On exit `rem` prints `Session saved. Resume with: rem --resume <id>`.

```
rem --help       # usage, config + session pointers (no TUI, no config needed)
rem --version    # print version and exit
rem --resume <id>  # resume a saved session
```

## Headless (`rem -p`)

One turn, no TUI — stdout stays pipeable (diagnostics go to stderr):

```bash
rem -p "explain src/main.rs"          # print reply to stdout
rem -p "summarize" --output json    # tools + result as one JSON doc
rem -p "summarize" --output jsonl   # one JSON object per line
echo "context..." | rem -p "use this"  # stdin (context) + -p (instruction)
rem -p "continue" --resume <id>     # chain onto a saved session
```

Rules: `-p` only (no `exec` subcommand).
`@path` refs and `/skill:name` invokes expand exactly like the TUI submit path.
Sessions persist per turn; resume hint goes to stderr so stdout pipes cleanly.
Exit codes: `0` turn ok, `1` turn/provider/save error, `2` CLI usage error.

## Slash commands

Full reference — type `/` for the floating menu, `Up`/`Down` to move, `Tab` to complete, `Enter` to run, `Esc` to dismiss:

| Command | Does |
|---|---|
| `/quit` | quit (same as `Ctrl+D` on empty input) |
| `/new` | start a new session (saves current, resets context, clears screen) |
| `/help` | list commands |
| `/theme [name]` | list themes, or switch live (`/theme gruvbox` narrows, `Tab` completes, active marked `●`) |
| `/init` | model inspects the repo and writes project-root `AGENTS.md` |
| `/resume [id]` | bare `/resume` opens the session picker (this project, newest first); `/resume <id>` jumps straight there |
| `/rename [name]` | rename current session |
| `/fork` | clone current history into a new session id, title reset |
| `/skills [reload]` | list discovered skills; `/skills reload` re-scans + rebuilds so new skills work without restart |
| `/skill:<name> [task]` | invoke a skill: body expands into the turn, transcript shows the short form. Up to 6 leading mentions stack; `$ARGUMENTS` passes the task text |
| `/model [[provider:]name]` | bare lists `provider:model` combos; `/model <name>` switches within the active provider, `/model <provider>:<name>` switches providers. History preserved, footer + session tag updated |
| `/effort [low\|medium\|high]` | bare shows the session level; with a level applies live to the active provider (session-local, config file untouched) |
| `/context` | local usage: message count (~chars), compaction trigger/keep window, summary status. No token API |

Keys: `Enter` submit, `Shift+Enter` or `Ctrl+O` newline, `Esc` interrupt a run (second `Esc` clears input), `Ctrl+C` clear, `j/k`, `Up/Down`, `Ctrl+d/u` in popups.

## File refs (`@` mentions)

Point at project files from the composer — content is inlined into the model payload, transcript keeps the short form:

- `@src/main.rs` inlines the file (capped at 2000 chars with a `[truncated N chars]` note)
- `@src/` inlines a non-recursive `file:`/`dir:` listing (500 entries, same shape as `list_directory`)
- `@src/main.rs:10-20` inlines lines 10–20 with line numbers; `@src/main.rs:42` one line; `@src/main.rs:100-` to end of file (end past EOF clamps)
- Multiple refs expand in appearance order: `compare @a.rs:1-10 with @b.rs`
- Bad refs block submit and name the ref: missing, unreadable, binary, outside-root (`../`, absolute, symlink escapes), invalid ranges (`20-10`, `0`, past-EOF start), range-on-directory
- `@` completes via fuzzy popup (`Tab`/`Enter` accept, range suffix preserved, `Esc` dismisses); `/`-menu wins on slash lines

## Skills (compact)

Claude-compatible `<name>/SKILL.md` bundles (YAML frontmatter `description` + Markdown body + sibling files). Identity is the folder name; malformed files load leniently (name-only + note) instead of breaking startup.

Discovery, first wins: `<project>/.agents/skills/` → `~/.config/rem/skills/` → `~/.agents/skills/`.

At startup the preamble gets an index (`name — description — path + siblings`); full bodies load on demand via `read`, scripts run via `bash`. No privileged runner, no new tool. See ADR-0013.

## Sessions (compact)

Persisted per turn in SQLite (`sessions.db` under your platform data dir), keyed by literal canonicalized cwd. One row: `id, project_root, title, model, messages_json`. The `model` column stores the `provider:model` tag.

- `rem --resume <id>` restarts anywhere; unknown ids exit non-zero
- `/resume` picker filters to the current project; `/fork` copies history verbatim
- Resume restores the stored tag: `/resume <id>` and the picker switch the live client when it differs (unknown tags warn and keep the current client + history)
- `/model` persists the new tag per switch; autosave keeps history + title — failures surface, never silent
- Titles fall back to the first user message (40 chars); autosave is synchronous — failures surface, never silent

## Themes (compact)

Every color comes from the active theme — no hardcoded colors in the render path. Pointer lives top-level in config (`theme = "gruvbox"`), files live in `~/.config/rem/themes/<name>.toml`. Bundled themes ship in `themes/`.

```bash
/theme            # list installed + current
/theme gruvbox    # load, apply live, save to config.toml byte-for-byte
```

Missing keys and bad hex fall back per-key to the built-in palette. See `docs/themes.md`.

## Tools

13 tools, wired through Rig's multi-step loop (up to 100 model/tool steps per turn):

`read`, `write`, `edit`, `bash`, `list_directory`, `git_status`, `git_diff`, `grep`, `glob`, `web_fetch` (URL → text), `web_search` (DuckDuckGo, no key), `todowrite`, `task`.

### Subagents (serial, v1)

`todowrite` owns the session task list. Pass the full list every call; each item is `content` + `status` (`pending` / `in_progress` / `completed`). Unknown fields (e.g. Claude-style `priority`) are accepted and ignored. `/todos` shows the list; `/new` clears it.

```
{"todos": [{"content": "write failing test", "status": "completed"}, {"content": "make it pass", "status": "in_progress"}]}
```

`task` spawns one scoped subagent and waits for its final summary:

```
{"description": "fix login retry", "prompt": "Fix the login retry bug in src/auth.rs. Read the file first, keep the change minimal, summarize what changed."}
```

Caps and abort semantics: serial only (one child at a time); the child gets a fresh context plus your prompt under the identical preamble, the same tools minus `task` (depth-1, no recursion), the same 100-turn / 30s-bash budget. The parent transcript shows a single `task` row (summary only, capped at 4000 chars) — inner steps are never streamed. `Esc` aborts the child with the turn. Parallel fan-out and background `/tasks` are deferred.

Project context: project-root `AGENTS.md` only (no walk-up, no global), appended verbatim after the preamble under `Project context from AGENTS.md:`. Missing file = silently skipped. Survives compaction, takes effect on next restart. History compacts past 100 messages via one-shot summary (tools off, no recursion) — failures restore history instead of dropping it.

## Limits

Explicit, because a learning project should not oversell:

- **No sandbox.** Shell commands run with your user privileges. No namespaces, seccomp, or kernel boundary.
- **No secrets redaction.** Reads return file bytes verbatim in v1.
- **Local single-user.** No remote/multi-user support.

## Architecture & docs

- `docs/adr/` — historical design records (permissions ADRs 0001–0003, 0011 describe the removed gate)
- `docs/glossary.md` — Principal / Skills, the living source of truth
- `docs/themes.md`, `docs/popup-test-matrix.md`

## Development

```bash
cargo test            # offline-green: skills, TUI regressions, agent loop (live web checks ignored)
cargo test -- --ignored  # opt-in: 2 live web checks (need network)
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Tests live next to code (`skills.rs`, `tui.rs`, `agent.rs`, `config.rs`, …).

## License

MIT — see `LICENSE`.
