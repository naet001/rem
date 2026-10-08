## Problem Statement

The rem agent lacks checkpoint/undo/rewind functionality. Without the ability to snapshot state before mutations, users have no way to recover from erroneous edits, tool outputs, or failed turns. Currently, if an `edit`, `write`, or `bash` command corrupts a file or produces undesired output, there is no mechanism to revert to a previous state. Users risk losing work or introducing bugs with no recourse except manual inspection and rework. The P0 checkpoints in TODO.md item 7 highlight this gap: snapshot before each mutate turn, implement `/undo` and `/rewind [n]`, provide a diff-accept flow, and build a fuzzy `apply_patch`-style edit tool.

## Solution

Implement a checkpoint system that snapshots the essential state before any mutating operation (`edit`, `write`, `bash`). The snapshot includes: (a) the `git diff` of uncommitted changes in the project root, and (b) the `messages_json` serialization of the conversation history. These snapshots are stored in a persistent, ordered log that the `/undo` and `/rewind [n]` commands can restore. Additionally, introduce a diff-accept flow for `edit` operations that presents the proposed patch to the user for explicit `[a]ccept` or `[r]eject` before the edit is permanently applied. Finally, enhance the `edit` tool to support an `apply_patch`-style fuzzy match where `oldText` is matched more flexibly (e.g., substring or regex-lite) rather than requiring an exact unique match.

## User Stories

1. As a rem user, I want a snapshot of `git diff` and `messages_json` to be automatically captured before each mutating tool call (`edit`, `write`, `bash`), so that I can restore the project and conversation state if something goes wrong.

2. As a rem user, I want to run `/undo` after a mutating operation, so that the most recent snapshot is restored (files reverted, conversation history rolled back to the state before that operation).

3. As a rem user, I want to run `/rewind n` to restore the state from n turns ago, so I can skip back multiple steps at once rather than repeated `/undo` invocations.

4. As a rem user, I want any mutating `edit` operation to show me a diff of the proposed change and ask for my explicit `[a]ccept` or `[r]eject` before the edit is landed, so I can review and cancel unintended modifications.

5. As a rem user, I want the `edit` tool to support fuzzy/partial `oldText` matching (similar to `apply_patch`), so that I can edit text even when I don't have the exact unique string, as long as the target text is recognizable.

6. As a rem user, I want the snapshot log to be persistent across sessions, so that `/undo` and `/rewind` work even after I restart rem.

7. As a rem user, I want the checkpoint system to be transparent—snapshots are taken automatically and I am not required to type any extra commands to enable them, so the protection is always active.

8. As a rem user, I want `/undo` and `/rewind` to restore the `messages_json` state as well as the file system state, so my conversation context is consistent with the file state after a rollback.

9. As a rem user, I want to see a clear diff preview when an `edit` is proposed (showing oldText vs newText highlights), so I can make an informed accept/reject decision.

10. As a rem user, I want the checkpoint system to have no observable performance impact during normal turns, so that snapshotting before mutating operations does not slow down the agent loop.

## Implementation Decisions

- **Snapshot seams**: The highest seam for snapshotting is the `AgentLoop::chat` method in `agent.rs`, which is the entry point for every turn. Before the LLM produces its tool calls, we snapshot `git diff` + `messages_json`. After the turn completes and edits are applied, we record the snapshot index. This means one seam covers all mutating operations.

- **Snapshot storage**: Snapshots are stored in a `Vec<(git_diff, messages_json, turn_index)>` within the `Context` struct (or a new `CheckpointStore` struct). The vector is bounded (e.g., keep last 50 snapshots) and pruned during compaction. Each snapshot is tagged with the turn number for `/rewind n` lookups.

- **`/undo` command**: When invoked, `/undo` pops the most recent snapshot from the log, runs `git checkout` (or `git diff` reverse patches) to restore files, and reconstructs the `messages_json` state by replacing the current `messages` Vec with the snapshot's serialized version. The turn counter is also rolled back.

- **`/rewind n` command**: Similar to `/undo` but restores the state from n turns ago. If n exceeds the available snapshot history, the oldest available snapshot is used. The user is shown a summary of what will be restored before confirmation.

- **Diff-accept flow for `edit`**: Before the `EditTool` writes the edit to a file, a preview diff is generated showing the `oldText` → `newText` replacement. The user is prompted in the TUI: `[a]ccept / [r]eject`. If accepted, the edit lands as normal. If rejected, no edit is applied and the turn continues. The diff preview uses the existing `patch_row` rendering from `tui.rs`.

- **Fuzzy `edit` tool enhancement**: The `EditTool` currently requires `oldText` to match exactly once in the file. The new mode supports a `--fuzzy` flag (or a config setting) where `oldText` is treated as a substring match: the first occurrence containing the substring is replaced. If multiple occurrences contain the substring, the user is disambiguated. This is inspired by `apply_patch`'s approach of matching by content rather than requiring uniqueness of the full string.

- **Module modifications**:
  - `src/context.rs`: Add `Checkpoint` struct (`git_diff: String`, `messages_json: String`, `turn_idx: usize`), and a `Vec<Checkpoint>` with bounded storage and pruning logic.
  - `src/agent.rs`: In the `chat` method, before the `with_retry` block, capture `git diff` via the `GitDiffTool` and `messages_json` via `export_messages_json`. After the turn, record the snapshot. Wire `/undo` and `/rewind n` as new agent commands.
  - `src/tools/edit.rs`: Add a fuzzy-match path where `oldText` is checked as a substring; if exactly one match is found, proceed; if multiple, return an error asking the user to disambiguate.
  - `src/main.rs` / `src/tui.rs`: Add `/undo` and `/rewind <n>` command handlers that interact with the checkpoint store and present TUI confirmation prompts.

- **API contracts**: 
  - New agent command `/undo` returns `Result<(), String>` — restores last snapshot or error if none exist.
  - New agent command `/rewind <n>` returns `Result<(), String>` — restores snapshot n turns ago.
  - `edit` tool may receive an optional `"fuzzy": true` flag in args; when present, `oldText` matching is relaxed to substring inclusion.

- **Technical clarifications**: 
  - The `git diff` snapshot captures uncommitted changes in the project root. If no git repo exists, an empty string is stored.
  - The `messages_json` snapshot is the serialized `Vec<Message>` from the context. On restore, it replaces the current messages Vec entirely.
  - Snapshot storage is bounded to prevent unbounded growth; a config constant `MAX_CHECKPOINTS = 50` is used, with the oldest snapshot dropped when the limit is exceeded.
  - The diff-accept flow is only triggered for `edit` tool calls, not for `write` or `bash`, since those have different semantics (write replaces the entire file, bash runs a command).

- **User-facing behavior**: 
  - On each mutating turn, the user sees a brief "snapshot saved" indicator in the TUI status line (optional, configurable).
  - After `/undo` or `/rewind`, the TUI displays a summary of what was restored (files reverted, messages truncated).
  - If the user rejects a diff-accept prompt, the edit is not applied and the turn's normal output is still produced (the LLM's response is not suppressed).

## Testing Decisions

- **Snapshot correctness**: Write a test that performs an `edit` tool call, then invokes `/undo`, and asserts that the file content reverts to the pre-edit state and the message count reverts accordingly.

- **`/undo` no-op**: Test that `/undo` when no snapshots exist returns a polite error message without crashing.

- **`/rewind n` bounds**: Test `/rewind 0` (no-op, current state), `/rewind 1` (restores one turn ago), and `/rewind > available_count` (clamps to oldest snapshot).

- **Diff-accept flow**: Integration test that an `edit` with `--flag fuzzy` or the default exact match presents a TUI prompt; verify that `[a]` lands the edit and `[r]` cancels it.

- **Fuzzy edit matching**: Test the `edit` tool with `oldText` as a substring that matches exactly one occurrence; verify the edit succeeds. Test with a substring that matches multiple occurrences; verify the tool returns an error asking for disambiguation.

- **Performance**: Benchmark that snapshot capture + recording adds < 10ms overhead per turn (acceptable since turns are already serialized).

- **Prior art**: The existing `take_for_compaction` / `apply_summary` flow in `context.rs` provides a pattern for draining and restoring messages that can be reused for checkpoint logic.

## Out of Scope

- Real-time, per-operation snapshot for every single tool call (only mutating tools `edit`, `write`, `bash` are snapshotted; read-only tools like `read`, `grep`, `bash` without file modifications are not included).
- Automatic conflict resolution when multiple snapshots overlap; `/undo` always restores the single most recent snapshot.
- Web UI or external dashboard for viewing checkpoint history; the feature is CLI/TUI-focused.
- Storing snapshots in an external database or file; snapshots exist in memory within the running rem session (though persisted to disk via the session's state JSON if the framework supports it).
- `/redo` command to re-apply a previously undone edit; scope is limited to `/undo` and `/rewind`.
- Changing the `edit` tool's default matching behavior (exact unique match remains the default; fuzzy is an opt-in flag).

## Further Notes

The checkpoint/undo/rewind system is the foundational P0 item that enables all higher-level features (parallel subagents, session branches, error recovery). Without it, users cannot safely experiment with edits or tool outputs. The implementation should prioritize correctness and minimal surprise: snapshots happen automatically, `/undo` is a simple revert, and the diff-accept flow is opt-in per `edit` call. The fuzzy `edit` enhancement is the most user-facing change and should be documented in the tool's help entry. This spec aligns with ADR-0012 (ref-files) and ADR-0015 (subagents) by extending the same snapshot/pattern approach to state recovery.