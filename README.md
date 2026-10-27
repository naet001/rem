<a name="readme-top"></a>

<div align="center">

# rem

**A terminal coding agent in Rust**

Full-screen Ratatui TUI, a Rig agent loop with 13 built-in tools, MCP servers,
parallel subagents, and session persistence in SQLite. Every tool call executes
immediately — no permission gates, no approval prompts, no modes.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/naet001/rem/dev.yml?branch=dev&label=CI)](https://github.com/naet001/rem/actions/workflows/dev.yml)
[![Rust 2024](https://img.shields.io/badge/rust-2024-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/)

[Getting started](#getting-started) · [Usage](#usage) · [Roadmap](#roadmap) · [Contributing](#contributing)

</div>

<hr />

<details>
<summary><strong>Table of Contents</strong></summary>

<ol>
<li><a href="#about-the-project">About The Project</a> — <a href="#what-it-does">What it does</a> · <a href="#safety-model-and-limitations">Safety model and limitations</a> · <a href="#built-with">Built With</a></li>
<li><a href="#getting-started">Getting Started</a> — <a href="#prerequisites">Prerequisites</a> · <a href="#installation">Installation</a></li>
<li><a href="#usage">Usage</a> — <a href="#configuration">Configuration</a> · <a href="#slash-commands">Slash commands</a> · <a href="#tools">Tools</a> · <a href="#subagents-and-todos">Subagents and todos</a> · <a href="#headless-mode">Headless mode</a> · <a href="#file-references">File references</a> · <a href="#skills">Skills</a> · <a href="#sessions">Sessions</a> · <a href="#themes">Themes</a> · <a href="#context-management">Context management</a></li>
<li><a href="#architecture">Architecture</a></li>
<li><a href="#roadmap">Roadmap</a></li>
<li><a href="#contributing">Contributing</a></li>
<li><a href="#license">License</a></li>
<li><a href="#contact">Contact</a></li>
<li><a href="#acknowledgments">Acknowledgments</a></li>
</ol>

</details>

## About The Project

`rem` is a coding agent that lives in your terminal. You describe work in plain
text; it reads and edits files, runs shell commands, searches the web, and
streams every step into the terminal's own scrollback. It is deliberately
direct: the agent is trusted with your shell, and the consequences of that choice
are documented below rather than hidden behind a prompt.

### What it does

- **Agent loop** — up to 100 model/tool steps per turn with 13 built-in tools.
- **MCP client** — stdio and streamable-HTTP servers from one JSON file, with an
  OAuth browser flow; their tools register as `mcp__<server>__<tool>`.
- **Parallel subagents** — fan out up to 8 scoped children, 4 at a time, or
  detach the whole call with `background: true` and get the summary later.
- **Turn rollback** — `/undo` and `/rewind` restore both files and conversation
  history from pre-turn checkpoints.
- **Sessions, skills, themes** — SQLite persistence per project, Claude-style
  `SKILL.md` bundles, and a theme file behind every color.
- **`@` file references and headless mode** — inline files, directories or line
  ranges into a prompt; run a single turn with `rem -p` and keep stdout pipeable.

### Safety model and limitations

Tool calls are unconditional — no gate, no dry-run, no allowlist anywhere in the
tool path. These are properties of the design, not gaps in it. Read them before
pointing `rem` at a repository you care about.

- **No sandbox.** `bash` runs with your user privileges. No namespaces, seccomp,
  or kernel boundary.
- **No path confinement in the tools.** `read`, `write`, `edit`,
  `list_directory`, and `bash` accept absolute paths and `..` traversal. Only `@`
  references typed into the composer are root-checked, and that check is lexical:
  `../` and absolute paths are rejected, but a symlink inside the project that
  points outside it is followed.
- **No secrets redaction.** Reads return file bytes verbatim.
- **No token accounting.** Context usage is a local character estimate.
- **Local, single-user.** No remote or multi-user support.
- **Subagent summaries are lossy** — capped at 4000 characters, inner steps
  never streamed.

### Built With

All of these are direct dependencies in `Cargo.toml`. `rusqlite` vendors SQLite
and `reqwest` runs on rustls, so nothing links against system OpenSSL.

[![Ratatui + Crossterm](https://img.shields.io/badge/ratatui%20%2B%20crossterm-0.29%20/%200.28-7fbbb3)](https://ratatui.rs/) [![Rig](https://img.shields.io/badge/rig-0.42-4ea8de)](https://docs.rs/rig)
[![rmcp + oauth2](https://img.shields.io/badge/rmcp%20%2B%20oauth2-2%20/%205-9ccfd8)](https://github.com/modelcontextprotocol/rust-sdk) [![Tokio](https://img.shields.io/badge/tokio-1-fab38e?logo=tokio)](https://tokio.rs/)
[![rusqlite](https://img.shields.io/badge/rusqlite-0.32%20bundled-cba6f7?logo=sqlite)](https://github.com/rusqlite/rusqlite) [![pulldown-cmark](https://img.shields.io/badge/pulldown--cmark-0.13-3e8fb0)](https://github.com/raphlinus/pulldown-cmark)
[![reqwest on rustls](https://img.shields.io/badge/reqwest%20%28rustls%20tls%29-0.12%20/%200.13-94e2d5)](https://github.com/seanmonstar/reqwest)

<p align="right"><a href="#readme-top">back to top</a></p><hr />

## Getting Started

### Prerequisites

Rust 1.85 or newer (edition 2024) and a C compiler. SQLite is compiled in through
`rusqlite`'s `bundled` feature and TLS is rustls, so there is no system libsqlite
or OpenSSL to install. You also need a reachable OpenAI-compatible endpoint or an
Anthropic key.

### Installation

```bash
cargo install --git https://github.com/naet001/rem

# or build from source
git clone https://github.com/naet001/rem && cd rem
cargo build --release          # target/release/rem
```

CI builds release binaries for `x86_64-unknown-linux-gnu` and
`aarch64-apple-darwin` when a version tag is pushed. No release has been
published yet, so install from source for now.

```bash
mkdir -p ~/.config/rem
cp config.example.toml ~/.config/rem/config.toml
export OPENAI_API_KEY="sk-..."   # must match the env: reference in config
rem
```

<p align="right"><a href="#readme-top">back to top</a></p><hr />

## Usage

Type a request, `Enter` to send, `/quit` to exit. On the way out `rem` prints
`Session saved. Resume with: rem --resume <id>`. Entry points: `rem` (TUI),
`rem --resume <id>` (reopen a saved session), `rem --help` (usage, config +
session pointers, no TUI), `rem --version`.

### Configuration

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

Keys accept `env:VAR` indirection only — a literal secret is rejected, so a config
is safe to paste into an issue. Every key, default and error message is in
[docs/configuration.md](docs/configuration.md); MCP servers live separately in
`~/.config/rem/mcp.json` ([docs/mcp.md](docs/mcp.md)).

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
| `/resume [id]` | bare opens the picker; `<id>` jumps straight to that session |
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
numbered red/green bands in the transcript. Per-tool arguments, caps and error
behavior: [docs/tools.md](docs/tools.md).

### Subagents and todos

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
batch. `background: true` detaches and returns an id immediately — `/tasks` shows
status and the summary arrives attached to a later turn. Children get fresh
context plus the same tools minus `task` and minus MCP, under the same 100-step
and 30-second budgets. `Esc` aborts foreground children with the turn; detached
work runs to completion.

### Headless mode

One turn, no TUI. Diagnostics go to stderr so stdout stays pipeable:

```bash
rem -p "explain src/main.rs"                 # reply on stdout
rem -p "summarize" --output json             # tools + result as one document
rem -p "summarize" --output jsonl            # one JSON object per line
echo "context..." | rem -p "use this"        # stdin context + instruction
rem -p "continue" --resume <id>              # chain onto a saved session
```

`@path` refs and `/skill:name` invokes expand exactly as they do in the TUI. Exit
codes: `0` turn ok, `1` turn/provider/save error, `2` CLI usage error. Background
tasks run inline here — there is no later turn to deliver to.

### File references

Point at project files from the composer; content is inlined into the payload
while the transcript keeps the short form.

```
@src/main.rs          file contents, capped at 2000 chars
@src/                 non-recursive file:/dir: listing, 500 entries
@src/main.rs:10-20    line range with line numbers (:42, :100- also work)
```

Multiple refs expand in appearance order. Bad refs block submit and name the ref:
missing, unreadable, binary, outside the project root (`../` or absolute),
invalid ranges (`20-10`, `0`, past-EOF start), or a range on a directory. `@`
completes via fuzzy popup; `/` wins on slash lines. The root check is lexical, so
a project symlink pointing outside the tree is still followed.

### Skills

Claude-compatible `<name>/SKILL.md` bundles — YAML `description` frontmatter,
Markdown body, sibling files. Identity is the folder name; malformed files load
leniently instead of breaking startup. Discovery order, first wins:
`<project>/.agents/skills/` → `~/.config/rem/skills/` → `~/.agents/skills/`. The
preamble receives an index (name, description, path, siblings); full bodies load
on demand through `read` and scripts through `bash`. No privileged runner, no
extra tool ([ADR-0013](docs/adr/0013-skills.md)).

### Sessions

Persisted per turn in SQLite — `sessions.db` under your platform data directory
(`REM_SESSIONS_DB` overrides), keyed by the literal canonicalized working
directory:

```
sessions(id, project_root, title, created_at, updated_at, model, messages_json)
```

`/fork` copies history verbatim, and the stored `provider:model` tag is restored
on resume — an unknown tag warns and keeps the current client. Autosave is
synchronous and failures surface rather than going silent. Titles fall back to the
first user message at 40 characters. The `/resume` picker opens on sessions for
the current project, newest first; press `a` to list every project in the database.

### Themes

`theme = "nord"` at the top level, files in `~/.config/rem/themes/<name>.toml`;
`/theme nord` loads it, applies live and saves the choice. Ten ship in
[`themes/`](themes): ashen, four catppuccin flavours, gruvbox dark and light,
nord, and tokyo-night day and night. Missing keys and malformed hex fall back
per-key to the built-in palette, and no hardcoded colors remain in the render path
— see [docs/themes.md](docs/themes.md).

### Context management

History compacts past 100 messages: everything older than the newest 40 is
replaced by a one-shot summary from the same model with tools disabled. On
summary failure the history is restored and the turn errors, rather than silently
dropping your conversation. Project `AGENTS.md` is appended to the preamble
verbatim and survives compaction. There is no token-counting API; `/context`
reports local message count and character estimates.

<p align="right"><a href="#readme-top">back to top</a></p><hr />

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

Design decisions are recorded as 21 dated ADRs in [`docs/adr/`](docs/adr). Note
that the permission-gate series (ADR-0001, 0002, 0003, 0011) describes a mechanism
that was removed: tool calls are unconditional now, and the ADRs stay as the
record of why. [`docs/glossary.md`](docs/glossary.md) is terminology's source of
truth.

Further reading: [tools](docs/tools.md) · [configuration](docs/configuration.md) ·
[MCP](docs/mcp.md) · [themes](docs/themes.md) · [contributing](docs/contributing.md)

<p align="right"><a href="#readme-top">back to top</a></p><hr />

## Roadmap

The core loop is usable day to day: tools, sessions, MCP, subagents and rollback
all work. The remaining clusters are streaming replies token-by-token instead of
at tool boundaries, a memory hierarchy above the single project-root `AGENTS.md`,
distribution (a published crate, packages, shell completions), and hardening —
sandboxing `bash` and redacting secrets on read. [`TODO.md`](TODO.md) keeps the
running gap analysis against Claude Code, Codex CLI and Pi as a working list
rather than a commitment; milestones are argued in [`docs/adr/`](docs/adr) first.

<p align="right"><a href="#readme-top">back to top</a></p><hr />

## Contributing

Pushes to `dev` must be `cargo fmt`, `cargo clippy --all-targets -- -D warnings`
and `cargo test` green — roughly 320 tests sit next to the code they cover, and
two live web checks are `#[ignore]`d so the suite passes offline. Setup, module
layout, the branch and release flow, and the conventions this repo holds to are in
[docs/contributing.md](docs/contributing.md); open an issue first for anything
beyond a small fix.

<p align="right"><a href="#readme-top">back to top</a></p><hr />

## License

Distributed under the MIT License. See [LICENSE](LICENSE) for more information.

<p align="right"><a href="#readme-top">back to top</a></p><hr />

## Contact

Nommoh Hayab — hnommoh@gmail.com · GitHub [@naet001](https://github.com/naet001)

Project: [https://github.com/naet001/rem](https://github.com/naet001/rem)

<p align="right"><a href="#readme-top">back to top</a></p><hr />

## Acknowledgments

- [Ratatui](https://ratatui.rs/) and [Crossterm](https://github.com/crossterm-rs/crossterm) — the entire TUI
- [Rig](https://docs.rs/rig) — the agent loop and tool calling
- [Model Context Protocol](https://modelcontextprotocol.io/) and [rmcp](https://github.com/modelcontextprotocol/rust-sdk) — MCP client support
- Anthropic's agent-skills format, which the `SKILL.md` bundle layout follows
- DuckDuckGo Lite — `web_search` works without an API key
- [Best-README-Template](https://github.com/othneildrew/Best-README-Template) — the shape of this README

<p align="right"><a href="#readme-top">back to top</a></p>
