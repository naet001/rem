# 02: Popup keeps range suffix on file completion

**What to build:** The caret-aware fuzzy file finder completes mid-line refs without destroying a typed `:range` suffix, so `@sr<TAB>` plus a range still delivers the slice from ticket 01. All existing popup behavior (filename-first ranking, 10 rows plus overflow, Up/Down move, Tab/Enter complete, Esc dismiss keeping text, slash-menu-wins precedence, busy and approval suppression) stays unchanged.

**Blocked by:** 01: Line-range slicing for file mentions.

**Status:** ready-for-agent

- [ ] Completing a file candidate preserves any typed `:range` suffix on the token
- [ ] Bare `@` still lists all project-relative paths with dirs trailing, respecting ignore and skip rules
- [ ] Fuzzy ranking still prefers filename hits, then path hits, then alphabetical
- [ ] Up/Down move selection, Tab/Enter completes (never submits), second Enter submits
- [ ] Esc dismisses keeping typed text and never aborts a run
- [ ] Slash menu still wins on slash-command lines; popup stays suppressed while busy or awaiting approval
- [ ] Covered by popup regression tests in the existing fuzzy-menu style plus one range-suffix preservation case
