# Checkpoints/Undo/Rewind Tickets

Based on the SPEC_CHECKPOINTS.md, breaking down the P0 checkpoints/undo/rewind feature into vertical slices.

## Ticket 01: Set up checkpoint storage structure
**Blocked by:** None (can start immediately)

**What it delivers:** A `Checkpoint` struct and bounded `Vec<Checkpoint>` storage in `Context` that can persist `git_diff` + `messages_json` + `turn_idx` for each snapshot, with automatic pruning at `MAX_CHECKPOINTS = 50`.

**Acceptance criteria:**
- [ ] `Checkpoint` struct defined with `git_diff: String`, `messages_json: String`, `turn_idx: usize` fields
- [ ] `Context` extends with `checkpoints: Vec<Checkpoint>` field (bounded, pruned on compaction)
- [ ] `new_checkpoint()` function creates a snapshot from current state
- [ ] `prune_checkpoints()` function drops oldest when exceeding max

**What to build:** From the user's perspective, the checkpoint system needs a place to live. This ticket establishes the data structure that all other tickets will build on. Without this, there's nowhere to store the snapshots.

---

## Ticket 02: Implement snapshot capture in AgentLoop::chat
**Blocked by:** 01

**What it delivers:** Automatic snapshot capture before each mutating turn in the `chat` method — capturing `git diff` via `GitDiffTool` and `messages_json` via `export_messages_json`, then recording the snapshot index after the turn completes.

**Acceptance criteria:**
- [ ] In `agent.rs` `chat` method, before `with_retry` block: capture `git diff` and `messages_json`
- [ ] After turn completes: create `Checkpoint` and push to `context.checkpoints` Vec
- [ ] Snapshot includes `turn_idx` for `/rewind n` lookups
- [ ] Bounded storage: oldest dropped when `len() > MAX_CHECKPOINTS`

**What to build:** The core mechanism that makes undo/rewind possible. Every mutating turn gets a safety net. From the user's perspective, this happens invisibly — they don't need to do anything special.

---

## Ticket 03: Implement /undo command
**Blocked by:** 02

**What it delivers:** The `/undo` agent command that pops the most recent snapshot from the log, runs `git checkout` (or reverse patches) to restore files, and replaces the current `messages` Vec with the snapshot's serialized `messages_json`. Turn counter also rolled back.

**Acceptance criteria:**
- [ ] `/undo` command handler in `agent.rs` 
- [ ] Returns error if no snapshots exist (no-op, no crash)
- [ ] Restores file state: `git checkout` of saved diff or apply reverse patches
- [ ] Restores conversation state: `messages` Vec replaced with snapshot's `messages_json`
- [ ] Turn index rolled back to snapshot's `turn_idx`
- [ ] TUI displays summary of what was restored

**What to build:** The primary recovery mechanism. After a bad edit or tool output, the user can say `/undo` and everything goes back to how it was before that turn. Critical for user confidence.

---

## Ticket 04: Implement /rewind n command
**Blocked by:** 02

**What it delivers:** The `/rewind <n>` agent command that restores the state from n turns ago. If n exceeds available history, clamps to the oldest snapshot. Shows a summary of what will be restored before confirmation.

**Acceptance criteria:**
- [ ] `/rewind <n>` command handler in `agent.rs`
- [ ] `n = 0` is a no-op (current state)
- [ ] `n > 0` restores snapshot at `current_turn - n` (clamped to available range)
- [ ] Shows summary before restoration: "Restore state from X turns ago? [y/N]"
- [ ] On confirmation: same restoration logic as `/undo` but from specified turn
- [ ] `n > available_count` uses oldest available snapshot

**What to build:** Fine-grained control. Instead of undoing one turn at a time, the user can skip back multiple steps. From the user's perspective, this is like a time machine for the agent session.

---

## Ticket 05: Implement diff-accept flow for edit tool
**Blocked by:** 02

**What it delivers:** Before the `EditTool` writes the edit to a file, a diff preview is generated showing `oldText` → `newText`. The TUI prompts: `[a]ccept / [r]eject`. If accepted, the edit lands. If rejected, no edit is applied.

**Acceptance criteria:**
- [ ] `edit` tool call generates diff preview before landing
- [ ] TUI shows: "Edit preview: oldText → newText. [a]ccept / [r]eject"
- [ ] `[a]ccept` lands the edit as normal
- [ ] `[r]eject` cancels the edit; turn continues without the modification
- [ ] Diff uses existing `patch_row` rendering from `tui.rs`
- [ ] User sees highlighted diff in the preview

**What to build:** User confidence for edits. Instead of an edit silently landing, the user reviews and approves. This is especially important since `edit` is one of the most commonly used mutating tools.

---

## Ticket 06: Enhance edit tool with fuzzy matching
**Blocked by:** 02

**What it delivers:** The `edit` tool supports an optional `"fuzzy": true` flag in args. When present, `oldText` matching is relaxed to substring inclusion: the first occurrence containing the substring is replaced. If multiple occurrences contain the substring, the user is disambiguated with an error.

**Acceptance criteria:**
- [ ] `EditArgs` extended with optional `fuzzy: bool` field
- [ ] When `fuzzy: true`: `oldText` checked as substring match in file content
- [ ] Exactly one match containing the substring → edit proceeds
- [ ] Multiple matches containing the substring → error asking for disambiguation
- [ ] No `fuzzy` flag (or `false`): exact unique match required (current behavior)
- [ ] Help text updated to document the `--fuzzy` / `"fuzzy": true` option

**What to build:** Edit flexibility. Users often don't have the exact unique text string handy. Fuzzy matching lets them match by a distinctive substring instead. This mirrors `apply_patch`'s approach of matching by content.

---

## Ticket 07: Add TUI command handlers for /undo and /rewind
**Blocked by:** 03, 04

**What it delivers:** Command handlers in `main.rs`/`tui.rs` that wire up `/undo` and `/rewind <n>` as interactive TUI commands. Presents confirmation prompts in the TUI style consistent with the rest of the application.

**Acceptance criteria:**
- [ ] `/undo` command registered in TUI command map
- [ ] `/rewind <n>` command registered, prompts for n value if not provided
- [ ] TUI shows confirmation: "Undo last mutate? This will restore files and conversation to state before the last mutating turn. [y/N]"
- [ ] TUI shows confirmation: "Rewind n turns? Restore state from N turns ago. [y/N]"
- [ ] On [y]es: invokes the corresponding agent command
- [ ] On [n]o: cancels, no state change
- [ ] TUI displays result summary after restoration

**What to build:** The user interface layer. All the backend logic is useless if the user can't access it from the TUI. These commands need to be reachable via the keyboard-driven TUI.

---

## Ticket 08: Write tests for checkpoint/undo/rewind flow
**Blocked by:** 01, 02, 03, 04, 05, 06

**What it delivers:** Comprehensive test suite covering the full checkpoint/undo/rewind workflow, ensuring correctness and preventing regression.

**Acceptance criteria:**
- [ ] Test: snapshot capture correctness (git diff + messages_json stored)
- [ ] Test: /undo no-op when no snapshots exist
- [ ] Test: /undo restores file state after edit
- [ ] /undo restores conversation state after edit
- [ ] Test: /rewind 0 is no-op
- [ ] Test: /rewind 1 restores one turn ago
- [ ] Test: /rewind > available uses oldest snapshot
- [ ] Test: diff-accept [a]ccept lands edit, [r]eject cancels
- [ ] Test: fuzzy edit matching (single match succeeds)
- [ ] Test: fuzzy edit multiple matches returns disambiguation error
- [ ] Performance benchmark: snapshot < 10ms overhead per turn

**What to build:** Quality assurance. These tests give confidence that the feature works end-to-end and catch regressions as the codebase evolves. The performance benchmark ensures the feature doesn't slow down the normal agent loop.

---

## Dependency chain overview

```
01 → 02 → 03 → 07
       ↘     ↘
        → 04 → (same 07)
              ↘
               → 05
                ↘
                 → 06
                  ↘
                   → 08
```

Tickets 03 and 04 both block 07 (the TUI handlers), but they can be worked on in parallel since they depend on the same foundation (02). Ticket 08 blocks everything and is the final verification step.

**Does this granularity feel right? Are the blocking edges correct: does each ticket only depend on tickets that genuinely gate it? Should any tickets be merged or split further?**