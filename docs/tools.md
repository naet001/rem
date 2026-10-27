# Tools

`rem` registers 13 built-in tools on one shared Rig tool server
(`build_tool_server`, `src/agent.rs:415`). Connected MCP servers add tools to
the same server at runtime, so a live session sees `13 + N` tools. See
[mcp.md](mcp.md).

Every tool call executes immediately. There is no approval gate, no dry-run
mode, and no permission prompt anywhere in the tool path.

## Summary

| Tool | Arguments | Limit |
|---|---|---|
| `read` | `path` (req), `offset`, `limit` | none |
| `write` | `path` (req), `content` (req) | creates parent dirs |
| `edit` | `path` (req), `edits[]` (req), `fuzzy` | each `oldText` unique |
| `bash` | `command` (req) | 30s, 8000 bytes |
| `list_directory` | `path` (req) | 500 entries, non-recursive |
| `git_status` | — | `(clean)` when empty |
| `git_diff` | `path`, `staged` | `(clean)` when empty |
| `grep` | `pattern` (req), `path` | 200 matches, 500 chars/line |
| `glob` | `pattern` (req) | 500 paths |
| `web_fetch` | `url` (req), `max_bytes` | 8000 bytes default, 30s |
| `web_search` | `query` (req), `count` | 5 results default, max 10 |
| `todowrite` | `todos[]` (req) | replaces the whole list |
| `task` | `description`, `prompt`, `tasks[]`, `background` | 8 children, 4 at a time |

## File tools

### `read`

`path` is required. `offset` is a 0-based line index and `limit` a line count;
both are optional, and omitting both returns the whole file. Output is the raw
file text — no line numbers, no truncation, no secrets redaction.

### `write`

Writes `content` to `path` and creates missing parent directories. The result
is `wrote {path} ({n} bytes)` followed by a unified diff of the change. The
previous contents are snapshotted first (missing file = empty snapshot), which
is what `/undo` restores from.

### `edit`

`edits` is an array of `{ oldText, newText }` pairs applied sequentially. Every
`oldText` must match **exactly once** in the file; a match count of 0 or 2+
fails the entire call, so no partial application happens. `fuzzy: true`
(default `false`) falls back to whitespace-insensitive line matching, with an
exact match still preferred when both are available. The result is
`applied {n} edit(s) to {path} ({n} bytes)` plus the unified diff.

### `list_directory`

Non-recursive, sorted, `file:` / `dir:` prefixed lines, capped at 500 entries
with a `[truncated to 500 entries]` note. An empty directory returns
`{path}: (empty directory)`.

## Shell and git

### `bash`

Runs `sh -c {command}` with your user's privileges. 30-second timeout, 8000-byte
combined-output cap (`[truncated to 8000 bytes]`). The result is
`exit: {code|signal}` followed by stdout, with stderr appended under a
`[stderr]` marker when present. A timeout is reported as a tool error.

### `git_status` / `git_diff`

`git_status` takes no arguments and returns `git status --short`, or `(clean)`.
`git_diff` accepts an optional `path` filter and a `staged` boolean
(`git diff [--staged] [-- {path}]`), also `(clean)` when empty. Both share the
bash timeout and output caps.

## Search

### `grep`

Matches a **literal substring, not a regular expression** — the pattern is
compared with `contains`. `path` defaults to `.`. Results are
`{file}:{line}: {text}` capped at 200 matches and 500 characters per line. It
skips `.git/`, `target/`, symlinks, and binary files (extension list plus a NUL
byte sniff). No hits return `no matches for {pattern} under {root} ({n} files)`.

### `glob`

Sorted matching paths, newline-joined, capped at 500 with
`[truncated to 500 paths]`. Skips `.git/` and `target/`. Patterns resolve from
the process working directory.

## Web

### `web_fetch`

Accepts only `http` and `https` URLs. `max_bytes` defaults to 8000 (minimum 1).
Non-2xx responses are errors carrying the status code. HTML is converted to
text: `<script>`/`<style>` bodies dropped, block tags become newlines, entities
decoded, blank lines collapsed. User-Agent is `rem-agent/0.1`. 30-second
timeout, no redirect-limit override.

### `web_search`

DuckDuckGo Lite via plain HTTP — no API key. `count` defaults to 5 and clamps
to 1..10. Results are numbered `{N}. {title}` / `{url}` / `{snippet}` blocks
with snippets cut to 300 characters and `uddg=` redirect URLs resolved back to
the target. Empty results return `no results for {query}`.

## Planning: `todowrite`

Each item is `{ content, status }` with status one of `pending`,
`in_progress`, `completed`. The call **replaces** the entire list — send the
full desired state every time. Unknown fields (for example Claude-style
`priority`) are accepted and ignored; an invalid `status` is a parse error.

The list lives in an `Arc<Mutex<Vec<TodoItem>>>` shared with the agent, is
rendered at the top of the TUI pane every frame, and is shown by `/todos`.
Marks are `○` pending, `◌` in progress, `●` completed. An empty array clears
the list. Nothing is persisted: `/new` and a restart drop it.

## Subagents: `task`

The description the model sees, verbatim:

> Spawn scoped subagent(s) and wait for the merged result. Single: description
> (short label) + prompt (full task). Fan-out: tasks[] array (up to 8, 4 run at
> a time) merged as ## sections in input order, per-child errors kept inline.
> background:true detaches the whole call and returns an id immediately; the
> summary arrives attached to a later turn (/tasks shows status). Children get
> fresh context, the same file/shell/git/search/web tools plus todowrite
> (never task: no recursion).

All four arguments are optional at the schema level; the exactly-one-shape rule
(`description`+`prompt` **xor** `tasks[]`) is enforced in `normalize()` because
JSON Schema cannot express XOR, and violations come back as model-visible
errors like "not both" and "must not be empty".

### Shapes

- **Single** — `description` + `prompt`. Returns the child's raw summary; a
  child error fails the tool call.
- **Fan-out** — `tasks[]` of `{ description, prompt }`, up to
  `FANOUT_MAX_TASKS = 8`. Over the cap the call is rejected so the model splits
  the batch. Up to `FANOUT_CONCURRENCY = 4` run at a time on a scoped
  `JoinSet`, the rest queue. Output is `## {description}` sections in input
  order; a failing child becomes `## {description} (error)` and does not sink
  the batch.
- **Background** — add `background: true` to either shape. The call detaches via
  `tokio::spawn` into a session-scoped store and returns
  `background {id} started ({n} subagent(s): {labels}); the merged summary
  will arrive attached to a later turn` immediately, with ids of the form
  `task-{N}`.

### Children

A child gets fresh context, its own todo list, and the 12 non-`task` built-ins.
It does **not** get MCP tools — `build_child_inner` uses its own `.tool()` chain
rather than the shared handle. Recursion is blocked by simply not registering
`task` on the child. Budgets match the parent: 100 model/tool steps and the same
30-second bash timeout.

### Abort semantics

Foreground children are awaited inside the tool call, so dropping the call
future drops the `JoinSet` and aborts every child. `Esc` therefore cancels a
fan-out batch with no extra plumbing. **`Esc` does not stop a background task** —
detached work is not tracked in the current turn, runs to completion, and its
summary is delivered later. Headless `-p` disables the background store, so
`background: true` runs inline there.

`/tasks` lists `{id} [{running|done}] {labels}`; `/tasks {id}` replays the
summary or reports `(still running)`. Completed, not-yet-reported summaries are
drained once per turn and prepended to your next prompt as
`background {id} finished ({labels}):\n{summary}`.

## Diff rendering

`write` and `edit` attach a unified diff built by `unified_diff`
(`src/tools/mod.rs:121`): `--- a/{path}` / `+++ b/{path}` headers, `@@` hunk
markers, 3 lines of context (`-U3`), with overlapping windows merged. Identical
content yields an empty diff and the summary line only. Line-ending or
trailing-newline-only changes produce a context tail annotated
`\ (only line endings or trailing newline differ)`. A full Myers `O(ND)` pass
runs only when the combined input is under 1000 lines; larger inputs get a
single coarse prefix/suffix-trimmed hunk.

## Output caps

Two independent budgets:

- Tool results entering history are cut to `Context::TOOL_BUDGET = 2000` chars
  with a `[truncated {n} chars]` note (`src/context.rs:109`).
- Subagent summaries are cut to 4000 chars with `… [truncated]`
  (`FULL_OUTPUT_CAP`, `src/agent.rs:284`).

Shell and web output carry their own 8000-byte caps listed above.

## Path handling

No file tool confines paths to the project root. `read`, `write`, `edit`,
`list_directory`, and `bash` accept absolute paths and `..` traversal, and child
processes inherit the process working directory. Root confinement exists only
for `@` file references typed into the composer
(`at_resolve_in_root`, `src/tui.rs:1003`) — and that check is **lexical**: it
normalizes `..` components away and rejects absolute paths, but never calls
`canonicalize`, so a symlink inside the project that points outside the tree is
still followed. This is deliberate; read
[the limitations](../README.md#safety-model-and-limitations).

## See also

- [ADR-0015 — serial subagents](adr/0015-serial-subagents.md)
- [ADR-0016 — parallel fan-out and background tasks](adr/0016-parallel-background-tasks.md)
- [glossary.md](glossary.md) — read/mutate/execute/network tool classes
