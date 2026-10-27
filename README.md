# rem

A terminal coding agent in Rust. Full-screen Ratatui TUI, a Rig agent loop with
13 built-in tools, MCP servers, parallel subagents, and session persistence in
SQLite. Every tool call executes immediately — no permission gates, no approval
prompts, no modes.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/naet001/rem/dev.yml?branch=dev&label=CI)](https://github.com/naet001/rem/actions/workflows/dev.yml)
[![Rust 2024](https://img.shields.io/badge/rust-2024-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/)

## What it does

- **Agent loop** — up to 100 model/tool steps per turn, streaming each resolved
  tool call into the terminal's own scrollback.
- **13 tools** — read, write, edit, bash, list, git status/diff, grep, glob,
  web fetch, web search, todowrite, task.
- **MCP client** — stdio and streamable-HTTP servers from one JSON file, with an
  OAuth browser flow. Their tools register as `mcp__<server>__<tool>`.
- **Parallel subagents** — fan out up to 8 scoped children, 4 running at a time,
  or detach the whole call with `background: true` and get the summary on a
  later turn.
- **Turn rollback** — `/undo` and `/rewind <n>` restore both files and
  conversation history from pre-turn checkpoints.
- **Sessions** — one SQLite row per session, keyed by project; resume, rename,
  and fork from a picker or the CLI.
- **Skills** — Claude-compatible `SKILL.md` bundles with a startup index and
  on-demand bodies.
- **`@` file refs** — inline a file, a directory listing, or an exact line range
  into the prompt.
- **Headless** — `rem -p "..."` runs one turn and keeps stdout pipeable, with
  `json` and `jsonl` output.
- **Themes** — every color comes from a theme file; switch live with `/theme`.
- **Two providers** — OpenAI-compatible endpoints and Anthropic, switchable
  mid-session with a unified `/effort` knob.

## Install

Requires Rust 1.85 or newer (edition 2024) and a C compiler. SQLite is compiled
in through `rusqlite`'s `bundled` feature and TLS is rustls, so there is no
system libsqlite or OpenSSL to install.

```bash
cargo install --git https://github.com/naet001/rem
```

Or build from source:

```bash
git clone https://github.com/naet001/rem
cd rem
cargo build --release      # target/release/rem
```

CI attaches release binaries for `x86_64-unknown-linux-gnu` and
`aarch64-apple-darwin` to each tagged release on
[the releases page](https://github.com/naet001/rem/releases).

Then point it at a model:

```bash
mkdir -p ~/.config/rem
cp config.example.toml ~/.config/rem/config.toml
# edit ~/.config/rem/config.toml: set active_provider + [providers.*] entries
export OPENAI_API_KEY="sk-..."       # match the env: reference in config
rem
```

## Configure

`~/.config/rem/config.toml` — one file, no search path, unknown keys rejected.

```toml
active_provider = "openai"
effort = "medium"                    # low / medium / high, mapped per provider

[providers.openai]
type = "openai-compatible"
base_url = "https://api.openai.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"

[providers.anthropic]
type = "anthropic"
key = "env:ANTHROPIC_API_KEY"
default_model = "claude-sonnet-4-5"
```

Keys accept `env:VAR` indirection only — a literal secret is rejected, so a
config is safe to paste into an issue. See
[docs/configuration.md](docs/configuration.md) for every key, default, and error
message.

Optional: MCP servers in `~/.config/rem/mcp.json`
([docs/mcp.md](docs/mcp.md)).

## Use it

Type a request, `Enter` to send, `/quit` to exit. On the way out `rem` prints
`Session saved. Resume with: rem --resume <id>`.

```
rem                        # TUI
rem --resume <id>          # reopen a saved session
rem --help                 # usage, config + session pointers, no TUI
rem --version
```

### Slash commands

Type `/` for the floating menu — `Up`/`Down` to move, `Tab` to complete, `Enter`
to run, `Esc` to dismiss. 17 registered commands, plus the `/skill:<name>`
invocation form:

| Command | Does |
|---|---|
| `/quit` | quit (also `Ctrl+D` on empty input) |
| `/new` | new session: saves current, resets context, clears screen |
| `/help` | list commands |
| `/init` | model inspects the repo and writes project-root `AGENTS.md` |
| `/model [[provider:]name]` | bare lists `provider:model` combos; with a name switches live, history preserved |
| `/effort [low\|medium\|high]` | show or set reasoning effort, session-local |
| `/theme [name]` | list themes, or switch live and save the choice |
| `/context` | message count, char estimate, compaction window, summary status |
| `/resume [id]` | bare opens the picker for this project; `<id>` jumps straight there |
| `/rename [name]` | rename the current session |
| `/fork` | clone history into a new session id |
| `/undo` | undo the last mutate turn — files and history |
| `/rewind [n]` | restore to the checkpoint from `n` turns ago, without consuming it |
| `/skills [reload]` | list discovered skills; `reload` re-scans without restart |
| `/skill:<name> [task]` | expand a skill body into the turn |
| `/mcp [on\|off\|login\|logout <name>]` | list, toggle, or authenticate MCP servers |
| `/todos` | show the session todo list |
| `/tasks [id]` | list background subagents, or show one summary |

Keys: `Enter` submit · `Shift+Enter` or `Ctrl+O` newline · `Esc` interrupt a run
(a second `Esc` clears input) · `Ctrl+C` clear · `j/k`, `Up/Down`, `Ctrl+d/u` in
popups.

### Tools

Registered on one shared tool server; MCP tools join it at runtime, so a live
session sees `13 + N` tools.

| Group | Tools |
|---|---|
| **Files** | `read` · `write` · `edit` · `list_directory` |
| **Shell + git** | `bash` · `git_status` · `git_diff` |
| **Search** | `grep` (literal substring) · `glob` |
| **Web** | `web_fetch` · `web_search` (DuckDuckGo, no key) |
| **Orchestration** | `todowrite` · `task` |

`write` and `edit` results carry a unified diff, rendered as a tinted patch with
numbered red/green bands in the transcript.

### Subagents

`todowrite` owns the session task list — state your understanding and plan, then
send the full list every call; each item is `content` plus `status` (`pending` /
`in_progress` / `completed`). Trivial one-step tasks skip it.

`task` spawns scoped children and waits for the merged summary:

```json
{"description": "fix login retry", "prompt": "Fix the login retry bug in src/auth.rs. Read the file first, keep the change minimal, summarize what changed."}
```

```json
{"tasks": [{"description": "audit deps", "prompt": "..."}, {"description": "audit CI", "prompt": "..."}], "background": true}
```

Fan-out runs up to 8 children bounded-parallel (4 at a time) and merges them as
`## <description>` sections in input order; one child failing does not sink the
batch. `background: true` detaches and returns an id immediately — `/tasks`
shows status and the summary arrives attached to a later turn.

Children get fresh context plus the same tools minus `task` and minus MCP, under
the same 100-step and 30-second budgets. `Esc` aborts foreground children with
the turn; detached background work runs to completion.

### Headless

One turn, no TUI. Diagnostics go to stderr so stdout stays pipeable:

```bash
rem -p "explain src/main.rs"                 # reply on stdout
rem -p "summarize" --output json             # tools + result as one document
rem -p "summarize" --output jsonl            # one JSON object per line
echo "context..." | rem -p "use this"        # stdin context + instruction
rem -p "continue" --resume <id>              # chain onto a saved session
```

`@path` refs and `/skill:name` invokes expand exactly as they do in the TUI.
Exit codes: `0` turn ok, `1` turn/provider/save error, `2` CLI usage error.
Background tasks run inline here — there is no later turn to deliver to.

### File refs

Point at project files from the composer; content is inlined into the payload
while the transcript keeps the short form.

```
@src/main.rs          file contents, capped at 2000 chars
@src/                 non-recursive file:/dir: listing, 500 entries
@src/main.rs:10-20    line range with line numbers (:42, :100- also work)
```

Multiple refs expand in appearance order. Bad refs block submit and name the
ref: missing, unreadable, binary, outside-root (`../`, absolute, symlink
escapes), invalid ranges (`20-10`, `0`, past-EOF start), or a range on a
directory. `@` completes via fuzzy popup; `/` wins on slash lines.

### Skills

Claude-compatible `<name>/SKILL.md` bundles — YAML `description` frontmatter,
Markdown body, sibling files. Identity is the folder name; malformed files load
leniently instead of breaking startup.

Discovery order, first wins: `<project>/.agents/skills/` →
`~/.config/rem/skills/` → `~/.agents/skills/`. The preamble receives an index
(name, description, path, siblings); full bodies load on demand through `read`
and scripts through `bash`. No privileged runner, no extra tool
([ADR-0013](docs/adr/0013-skills.md)).

### Sessions

Persisted per turn in SQLite — `sessions.db` under your platform data directory
(`REM_SESSIONS_DB` overrides), keyed by the literal canonicalized working
directory:

```
sessions(id, project_root, title, created_at, updated_at, model, messages_json)
```

`/resume` filters to the current project, `/fork` copies history verbatim, and
the stored `provider:model` tag is restored on resume — an unknown tag warns and
keeps the current client. Autosave is synchronous and failures surface rather
than going silent. Titles fall back to the first user message at 40 characters.

### Themes

`theme = "nord"` at the top level, files in `~/.config/rem/themes/<name>.toml`.
Ten ship in [`themes/`](themes): ashen, four catppuccin flavours, gruvbox dark
and light, nord, and tokyo-night day and night.

```bash
/theme         # list installed + current
/theme nord    # load, apply live, save to config.toml
```

Missing keys and malformed hex fall back per-key to the built-in palette. No
hardcoded colors remain in the render path — see [docs/themes.md](docs/themes.md).

### Context

History compacts past 100 messages: everything older than the newest 40 is
replaced by a one-shot summary from the same model with tools disabled. On
summary failure the history is restored and the turn errors, rather than
silently dropping your conversation. Project `AGENTS.md` is appended to the
preamble verbatim and survives compaction.

There is no token-counting API; `/context` reports local message count and
character estimates.

## Architecture

```
main.rs ── CLI, TUI entry
  ├── agent.rs      Rig loop, preamble, checkpoints, compaction
  │     └── tools/  13 built-ins + shared diff/error helpers
  ├── mcp.rs        MCP client: stdio + HTTP, OAuth, dynamic registration
  ├── tui.rs        ratatui frontend: commands, submit path, transcript
  │     └── popup.rs, history.rs, markdown.rs, theme.rs
  ├── context.rs    history, tool-result budget, undo checkpoints
  ├── sessions.rs   SQLite persistence
  ├── config.rs     providers, keys, effort
  ├── skills.rs     SKILL.md discovery + index
  └── headless.rs   -p single-turn mode
```

Design decisions are recorded as 21 dated ADRs in
[`docs/adr/`](docs/adr). Note that the permission-gate series (ADR-0001, 0002,
0003, 0011) describes a mechanism that was removed: tool calls are unconditional
now, and the ADRs stay as the record of why. [`docs/glossary.md`](docs/glossary.md)
is the source of truth for terminology.

Further reading: [tools](docs/tools.md) ·
[configuration](docs/configuration.md) · [MCP](docs/mcp.md) ·
[themes](docs/themes.md) · [contributing](docs/contributing.md)

## Limitations

Stated plainly, because these matter before you point it at a repo you care
about:

- **No sandbox.** `bash` runs with your user privileges. No namespaces, seccomp,
  or kernel boundary.
- **No path confinement in the tools.** `read`, `write`, `edit`,
  `list_directory`, and `bash` accept absolute paths and `..` traversal; only
  `@` references typed into the composer are root-checked. There is no approval
  step to catch a bad path.
- **No secrets redaction.** Reads return file bytes verbatim.
- **No token accounting.** Context usage is a local character estimate.
- **Local, single-user.** No remote or multi-user support.
- **Subagent summaries are lossy** — capped at 4000 characters, inner steps
  never streamed.

## Development

```bash
cargo test                 # offline-green (2 live web checks ignored)
cargo test -- --ignored    # opt-in: needs network
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

~320 tests live next to the code they cover. Pushes to `dev` run all three, then
wait for an explicit approval before promoting to `main` and cutting a release —
see [docs/contributing.md](docs/contributing.md).

## License

MIT — see [LICENSE](LICENSE).
