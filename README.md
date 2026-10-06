# rem

A terminal coding agent built as a personal learning project. Rust + Ratatui TUI, Rig agent loop, and a single permission gate that decides every tool call before it runs.

## Purpose

`rem` is a simple harness for learning how coding agents are put together. It is not production software: expect rough edges, and read the safety story below as a study, not a guarantee.

The project works through a few questions end to end:

- How do you route every model tool call through one inspectable choke point instead of scattering checks through each tool?
- How do you build a full-screen TUI — panes, popups, themes, keymaps — in Rust with Ratatui?
- How do you run an agent loop (Rig) where an approval pauses the run mid-stream and resumes the same run?
- How do sessions, skills, and config persist without any of it becoming privileged or magic?

## Design notes

The answers, as implemented:

- **One choke point** — `PermissionHook::on_tool_call` (ADR-0001) classifies every call. Tools stay dumb.
- **Pure policy** — `classify(tool, args, root) -> Allow | Confirm | Deny`. No I/O, fully unit-tested (ADR-0002).
- **Interrupt, don't kill** — approvals park the worker mid-run over mpsc + oneshot, then resume the same run (ADR-0003).
- **Modes are presets, not personas** — 5 hard-coded modes, session-local, auditable (ADR-0011).
- **Reusable knowledge** — Claude-compatible `SKILL.md` bundles with startup index + on-demand bodies (ADR-0013).
- **Decisions on record** — 18 ADRs in `docs/adr/`, 211 tests, `cargo test` green (minus 2 live web checks that need network).

## Permission verdicts

Three outcomes, no ambiguity:

| Class | Tools / patterns | Default |
|---|---|---|
| **Read → Allow** | `read`, `list_directory`, `glob`, `grep`, `git_status`, `git_diff` | Runs automatically, every mode |
| **Mutate → Confirm, with fast-path** | `write`, `edit` | Asks, **except** in `auto`/`edit` when ALL hold: inside project root, not sensitive (`.env`, `*.pem`, `*secret*`, `*credential*`, `~/.ssh`), under 32 KiB |
| **Execute → safe-list or ask** | `bash` (`sh -c`, 30s timeout) | Auto only for read-only introspection: `ls echo printf cat head tail wc pwd true uname date whoami which file stat basename dirname` + read-only `git status/diff/log/show/branch/remote/stash/tag`. Everything else (`cargo test`, `cargo build`, `rm`, `git push`, redirects, `sudo`, …) asks |
| **Network → Confirm** | `web_fetch`, `web_search`, plus network verbs inside `bash` (`curl`, `wget`, `ssh`, `pip install`, …) | Asks |
| **Annihilators → Deny** | `rm -rf /`, `rm -rf ~`, `mkfs … /dev/`, `dd … of=/dev/`, fork bomb, `curl … \| sh` | Blocked outright, no prompt, model gets replan feedback |
| **Unknown → Confirm** | any future tool | Fail-closed to approval, never auto |

Quoted strings are stripped before matching, so `echo "rm -rf /"` stays `Allow`. Matching is syntactic — see Limits.

Approval gives you four moves: **Allow once / Allow always** (session rule scoped to exact tool + args, never prefix) **/ Reject** (model replans in the same run) **/ Abort** (stops the turn). Approving with a note hides it from chat but shows it to the model on the next tool result.

## Permission modes

`Shift+Tab` cycles `plan → manual → auto → edit → yolo`. Session-local, shown in the footer, applies to the next message. In-progress runs keep their policy.

| Mode | What it does |
|---|---|
| `plan` | Read-only. Anything else is denied with model-visible feedback so the agent replans |
| `manual` (default) | Reads auto. Every `write`/`edit` asks, even small in-project ones. Bash safe-list auto, rest asks. Network asks |
| `auto` | Reads auto. Small in-project non-sensitive writes/edits auto-approve via fast-path. Bash safe-list auto, rest asks. Network asks |
| `edit` | Same gating as `auto` (small in-project edits auto, everything else gated). Intent: trust the normal dev loop while keeping high-impact ops gated — see ADR-0011 |
| `yolo` | Skips approvals except the safety floor: `bash` is still classified, annihilators still denied |

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

Talk, approve when asked, quit with `/quit`. On exit `rem` prints `Session saved. Resume with: rem --resume <id>`.

```
rem --help       # usage, config + session pointers (no TUI, no config needed)
rem --version    # print version and exit
rem --resume <id>  # resume a saved session
```

## Slash commands

Full reference — type `/` for the floating menu, `Up`/`Down` to move, `Tab` to complete, `Enter` to run, `Esc` to dismiss:

| Command | Does |
|---|---|
| `/quit` | quit (same as `Ctrl+D` on empty input) |
| `/new` | start a new session (saves current, resets context, clears screen) |
| `/help` | list commands |
| `/theme [name]` | list themes, or switch live (`/theme gruvbox` narrows, `Tab` completes, active marked `●`) |
| `/init` | model inspects the repo and writes project-root `AGENTS.md` via the normal write/approval flow |
| `/resume [id]` | bare `/resume` opens the session picker (this project, newest first); `/resume <id>` jumps straight there |
| `/rename [name]` | rename current session |
| `/fork` | clone current history into a new session id, title reset |
| `/skills [reload]` | list discovered skills; `/skills reload` re-scans + rebuilds so new skills work without restart |
| `/skill:<name> [task]` | invoke a skill: body expands into the turn, transcript shows the short form. Up to 6 leading mentions stack; `$ARGUMENTS` passes the task text |
| `/model [[provider:]name]` | bare lists `provider:model` combos; `/model <name>` switches within the active provider, `/model <provider>:<name>` switches providers. History preserved, footer + session tag updated |
| `/effort [low\|medium\|high]` | bare shows the session level; with a level applies live to the active provider (session-local, config file untouched) |
| `/context` | local usage: message count (~chars), compaction trigger/keep window, summary status. No token API |

Keys: `Enter` submit, `Shift+Enter` or `Ctrl+O` newline, `Esc` interrupt a run (second `Esc` clears input), `Ctrl+C` clear, `Shift+Tab` cycle mode, `j/k`, `Up/Down`, `Ctrl+d/u` in popups.

## Skills (compact)

Claude-compatible `<name>/SKILL.md` bundles (YAML frontmatter `description` + Markdown body + sibling files). Identity is the folder name; malformed files load leniently (name-only + note) instead of breaking startup.

Discovery, first wins: `<project>/.agents/skills/` → `~/.config/rem/skills/` → `~/.agents/skills/`.

At startup the preamble gets an index (`name — description — path + siblings`); full bodies load on demand via `read` (always `Allow`, auditable in the work tree), scripts run through normal `bash` approvals. No privileged runner, no new tool. See ADR-0013.

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

11 tools, wired through Rig's multi-step loop (up to 100 model/tool steps per turn):

`read`, `write`, `edit`, `bash`, `list_directory`, `git_status`, `git_diff`, `grep`, `glob`, `web_fetch` (URL → text), `web_search` (DuckDuckGo, no key).

Project context: project-root `AGENTS.md` only (no walk-up, no global), appended verbatim after the preamble under `Project context from AGENTS.md:`. Missing file = silently skipped. Survives compaction, takes effect on next restart. History compacts past 100 messages via one-shot summary (tools off, no recursion) — failures restore history instead of dropping it.

## Limits

Explicit, because a learning project should not oversell:

- **Heuristic, not sandbox.** Bash classification is syntactic (`; && || | & $( )`, quote-stripping). No namespaces, seccomp, or kernel boundary. A determined prompt-injection can shape a command the parser misreads — the human is still the boundary.
- **No secrets redaction.** Reads return file bytes verbatim in v1.
- **Local single-user.** The approver is you in the TUI. No remote/multi-user approver, no headless allow (fail-closed to deny when no TUI listens).
- **`auto`/`edit` still ask a lot.** `cargo test`, `cargo build`, `git push`, redirects, and all network access prompt by design. Use **Allow always** for repeatables.

## Architecture & docs

- `docs/adr/` — 0001 choke point, 0002 tiers, 0003 interrupt/resume, 0004–0005 Aster-style panes, 0006 keymap, 0007 slash menu, 0008 busy verbs, 0009 AGENTS.md + theme pattern, 0010 sessions + theme suggestions, 0011 modes + popups, 0012 at-mentions + vim navigation, 0013 MCP client + skills
- `docs/glossary.md` — Principal / Gate / Policy / Approver / Verdicts / Skills, the living source of truth
- `docs/themes.md`, `docs/popup-test-matrix.md`

Hook order matters: recorder first (observe-all), gate second (steer). `Skip` keeps the run alive for replanning; `Stop` is reserved for user abort.

## Development

```bash
cargo test            # offline-green: policy units, hook round-trips, skills, TUI regressions (live web checks ignored)
cargo test -- --ignored  # opt-in: 2 live web checks (need network)
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Tests live next to code (`permissions.rs`, `skills.rs`, `tui.rs`, `agent.rs`, `config.rs`, …). The classifier is pure — no I/O, no channels — so policy changes are test-first friendly.

## License

MIT — see `LICENSE`.
