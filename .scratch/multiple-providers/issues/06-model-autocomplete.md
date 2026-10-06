# 06: Model autocomplete popup mirroring theme

**What to build:** Typing `/model <partial>` mid-input opens a suggestions popup listing `provider:model` combos filtered by prefix, the way `/theme <partial>` pops up theme names. Tab/Enter completes the highlighted combo into the input; unmatched partials fall through to normal dispatch (which errors safely without side effects).

**Blocked by:** 04: Model and effort dispatch with footer.

**Status:** ready-for-agent

- [ ] Parser mirrors the theme-arg rules: `/model <partial>` with exactly one whitespace run, partial carrying no whitespace, `/` at start or after space; bare `/model ` lists everything; bare `/model` belongs to the command menu; multi-token args return no popup
- [ ] Filtering is prefix, case-sensitive on the full `provider:model` combo strings (same semantics as theme matching)
- [ ] Candidates come from a cache on App snapshotted at startup and refreshed after every `/model` switch (mirrors the skill_names pattern); no agent access on the keystroke path
- [ ] New menu variant flows through row-count, render, Tab-completion, and Enter-accept paths; `/`-wins precedence preserved (command menu first, theme/model/skill arg modes mutually exclusive)
- [ ] Submit resolves the highlighted combo before dispatch when suggestions are open (mirrors theme-arg accept)
- [ ] Covered by StubAgent tests: parser units, prefix filter units, live-menu open/filter/complete/accept, unmatched-closes, cache refresh on switch
