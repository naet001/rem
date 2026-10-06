# 05: Context readout with session persistence and docs

**What to build:** A user can check local context usage, and the active `provider:model` tag survives startup, resume, Picker selection, Fork, and Autosave, with the example config, slash reference, and help text updated.

**Blocked by:** 04: Model and effort dispatch with footer.

**Status:** ready-for-agent

- [ ] `/context` reports local message count versus the compaction trigger, character estimate, keep window, and last-summary status; no provider token API or price table
- [ ] The Session record stores the `provider:model` tag; startup, resume-by-id, Picker resume, Fork, and per-turn Autosave all carry it so the right client is rebuilt
- [ ] Example configuration shows the multi-profile shape with active pointer and `env:` keys
- [ ] Slash reference and help output document `/model`, `/effort`, and `/context`
- [ ] Format check, linter with warnings denied, and the full test suite are green
