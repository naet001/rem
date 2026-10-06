# 03: Discoverability and tracker truth for subagents v1

**What to build:** A new user can discover `todowrite` + `task` serial delegation from inside the app and the README, and the parity tracker no longer claims single-loop-only. No tool or Policy behavior changes.

**Blocked by:** 02: Serial Task subagent.

**Status:** done

- [x] In-app help notice documents the `todowrite` content+status shape and the `task` description+prompt shape plus the v1 limits (serial only, fresh-context child, shared approval queue, no recursion)
- [x] README documents serial delegation with copy-paste examples plus caps and abort semantics
- [x] Domain glossary entries and the decision record note serial subagents as landed with parallel fan-out and background `/tasks` still deferred
- [x] Parity tracker section 6 checks off `TodoWrite` and serial `Task`; only bounded-parallel merge and background polling remain as later slices
- [x] Verified by help-text assertion plus manual README and tracker parity check (no automated README test)
