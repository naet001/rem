# Glossary (rem)

> Living doc. Updated as grilling resolves. Source of truth for terms used in ADRs and code.
> Permission features (Gate / Policy / Approver / Verdicts / modes) were removed: every tool call runs immediately.

## Core

- **Principal**: The LLM-driven agent loop (`RigAgent`).
- **Run**: One `RigAgent::chat()` invocation, which internally may take up to 100 model/tool steps.

## Tool classes

- **Read**: `read`, `list_directory`, `glob`, `grep`, `git_status`, `git_diff`. Caveat: secrets redaction is out of scope for v1.
- **Mutate**: `write`, `edit`.
- **Execute**: `bash` (`sh -c`, 30s timeout).
- **Network**: `web_fetch`, `web_search` plus `bash` network verbs (`curl`, `wget`, `ssh`, `nc`, `pip install`, `cargo fetch`, `apt-get`).

## Popups (ADR-0011, ADR-0012)

- **Popup shell**: Shared centered modal surface shown inside a full-height managed viewport while open. It provides common layout, styling, selection, footer, and sizing behavior without prescribing content semantics; closing it restores the compact composer viewport.
- **Popup controller**: Feature-specific state and actions layered over the shared shell, such as resume or a future question prompt.
- **Inline filter**: Popup-owned filter mode rendered inside the shell. `/` enters it; `Esc` leaves filter mode before any popup dismissal policy is applied.
- **Popup navigation**: `j/k` and Up/Down move one item; `Ctrl+d/u` move half a visible page. `g/G` are not supported. `q` is reserved for popup-specific quit/abort semantics.

## Message list (ADR-0005, Aster-style)

- **Scrollback model**: Finished rows print into the terminal's own scrollback and are never touched again. Only the bottom pane stays managed. No Tab selection, no expand/collapse, no click-toggle, no in-app scroll.
- **Rendered Markdown reply**: Assistant reply text parsed as Markdown for headings, nested lists, inline emphasis/code, links, fenced code blocks, blockquotes, and tables. Prompts and tool output remain plain text.
- **Markdown link**: Rendered as its label followed by its destination in parentheses.
- **Markdown table**: Columns are sized to fit the available transcript width; cell contents wrap within those columns.
- **Markdown code block**: Fenced code rendered with preserved line breaks, indentation, and a subtle background.
- **Chapter mark**: The user row — filled band (`rail_bg`) with accent left rail (`▌`) + `❯ ` prompt. The only banded row; not a bullet.
- **Bullet row**: Every other row — dim `• ` glyph with hanging indent (continuations indent under the bullet, 2-column gutter).
- **Tool row**: Flat, one group per tool call. Bold label line + dim nested sub-rows, `└` branch on the first sub-row.
- **Patch row**: `▸ verb path` + `+N −M` counts pushed right, then a tinted body: full-row add/del background bands with a darker mark glyph.
- **Elision**: Long tool output renders first 4 + last 4 lines with a gap marker. No expand; the tail is always visible.
- **Hidden thinking**: Reasoning recorded internally, never printed. No marker row.
- **Live stream**: Each resolved tool prints immediately into scrollback.
- **Trailer**: `• Done ({elapsed}s · N tools)`. File counts, `+/-`, cost are later slices.

## Sessions (ADR-0010)

- **Session**: One persisted conversation row in `sessions.db` (`id, project_root, title, created, updated, model, messages_json`). Created at startup, resumed via `--resume <id>` or `/resume`.
- **Picker**: Bottom popup listing project sessions by `updated_at DESC`. Opened by bare `/resume`; `/resume <id>` bypasses it.
- **Fork**: Clone of the current session's `messages_json` into a new session id. History verbatim, title reset.
- **Status row**: 1 row, blank while idle; the slash menu may reclaim it. Busy = braille spinner + `{verb} · {s}s · esc to interrupt`, where `{verb}` cycles sequentially through 10 verbs (`working, thinking, cooking, pondering, reasoning, crafting, brewing, scheming, conjuring, noodling`), advancing every 2s and restarting at `working` each task (ADR-0008).
- **Autosave**: Per-turn persist via `blocking_lock` `export_sync` -> `save_messages`. Sync TUI submit path; failures must surface, never silent.
- **Project key**: Literal canonicalized cwd string stored as `project_root`. Picker filters by exact match (`list_for_project`), no prefix/glob.

## Skills (ADR-0013)

- **Skill**: Claude-compatible `<name>/SKILL.md` bundle (YAML frontmatter + Markdown body + optional sibling files). Identity is the folder name; frontmatter `name` ignored for v1.
- **Discovery roots**: `<project>/.agents/skills/` → `~/.config/rem/skills/` → `~/.agents/skills/`, first wins on name collision. Missing roots skipped.
- **Preamble index**: startup list of `name — description — SKILL.md path (+ bundle siblings)` after `AGENTS.md`. Bodies load on demand via `read`; scripts run via `bash`.
- **Invoke**: `/skill:<name> [task]` colon form; leading stacked mentions (up to 6) expand bodies in order with `$ARGUMENTS` substitution. Transcript shows the short form.
- **Reload**: `/skills reload` re-scans + rebuilds the inner agent so normal chat auto-knows new skills; history preserved, fail-closed on error. Refused while busy.
- **Lenient load**: missing `description` → first body line; bad YAML → name-only with note, direct invoke still works.

## Subagents (ADR-0015, ADR-0016)

- **TodoWrite**: `todowrite` tool owning the session task list (full-list replace; `content` + `status` `pending` / `in_progress` / `completed`). For medium-complex or larger tasks the agent first states its understanding + plan in the reply, then breaks the work into todos (trivial one-step tasks skip the list). Agent-shared state, never persisted; `/todos` shows it, `/new` clears it.
- **Task**: `task` tool spawning scoped subagent(s): single (`description` label + `prompt`) or fan-out (`tasks[]`, up to 8, 4 at a time, merged as `##` sections in input order). Fresh context per child, identical preamble + delegation note, same tools minus `task` (depth-1, no recursion). Parent transcript shows one row.
- **Fan-out**: bounded-parallel `tasks[]` batch on a scoped `JoinSet` (drop-the-future aborts all, so Esc works with zero TUI changes). Partial merge: per-child `## <label> (error)` blocks, never whole-batch failure.
- **Background**: `background:true` detaches the call into the session `BackgroundStore` (`task-N` ids, session-memory only, `/new` clears, headless degrades to inline). Result returns via TUI inject-on-next-submit; `/tasks` lists, `/tasks <id>` shows the summary.

## Non-goals (v1)

- True OS sandboxing (namespaces, seccomp, grsecurity).
- Secrets redaction on read.
- Multi-user / remote operation.

## Project instructions (ADR-0009)

- **AGENTS.md**: Verbatim project instructions loaded once at startup from `<project_root>/AGENTS.md` only (no walk-up, no global file, no cap). Appended after the static preamble under a `Project context from AGENTS.md:` header; silently skipped when missing. Outranks history, survives compaction, takes effect on next restart.
- **/init**: Slash command that rewrites to a fixed instruction and runs a normal agent turn — the model inspects the repo and overwrites project-root `AGENTS.md`. Transcript shows `/init`.

## File mentions (ADR-0012)

- **@ mention**: An inline file reference in composer input. `@file` inlines content; `@dir` inlines a listing; `@file:10-20` (also `@file:42`, `@file:100-`) inlines numbered lines. Multiple per message, in `@` appearance order.
- **@ trigger**: `@` starts a ref unless the char directly before it is an ASCII letter. `user@example.com` stays literal; `(@a.rs)` and `foo/@a.rs` trigger. No `@@` escape in v1.
- **@ token**: Text from a triggering `@` to whitespace, with trailing `, . : ; ! ? ) ] }` trimmed. No quoted-space paths in v1.
- **@ popup**: Caret-aware fuzzy file finder cloned from the `/` menu (subsequence match + ranked: filename hits first, then path, then alpha; `Up`/`Down` move, `Tab`/`Enter` complete, `Esc` dismiss, 10 rows + `+N more`). The `/` menu wins on `/command` lines. Filesystem walk, project-relative paths, dirs with trailing `/`, `.gitignore`-respected, `.git`/`target` skipped. Completion preserves a typed `:range` suffix; filtering ignores it.
- **Display-vs-send**: Transcript user row, input history, and title fallback keep the short original with `@` tokens; only the model payload appends `path:\n```\ncontents\n``` ` (files), `path:START-END:\n```\nN: line\n``` ` (ranges), or `dir/ (directory listing):\nfile:/dir: lines` (dirs) blocks. Sessions store the expanded prompt.
- **@ caps**: Per-file `TOOL_BUDGET` (2000 chars, slices included) with ` [truncated N chars]`; dirs non-recursive, sorted, `file:`/`dir:` lines, 500 entries (`list_directory` parity). Ranges: `END` past EOF clamps.
- **@ block**: Submit is blocked with a notice naming the bad ref when a ref is missing, unreadable, binary, outside the project root, or an invalid range (`END` before `START`, `0`, `START` past EOF, range on a directory). Large refs truncate, never block.
- **@ confinement**: Every ref resolves against the project root. The check is lexical (`at_resolve_in_root`): absolute paths and `../` escapes are blocked, but symlinks are never resolved, so a link inside the project pointing outside it is followed.
- **@ reads**: `@` only adds prompt context, never writes or executes.
