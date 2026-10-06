# 03: Discoverability and tracker truth for file mentions

**What to build:** A new user can discover `@file`, `@dir/`, `@file:line-range`, multi-ref, caps, and block behavior from inside the app and the README, and the parity tracker no longer claims landed work is missing. No expansion or popup behavior changes.

**Blocked by:** 02: Popup keeps range suffix on file completion.

**Status:** ready-for-agent

- [ ] In-app help notice documents `@file`, `@dir/`, `@file:line-range` forms, multi-ref appearance order, and blocked-submit naming the bad ref
- [ ] README documents `@` mentions with copy-paste examples covering file, directory, and line-range plus caps and block semantics
- [ ] Domain glossary file-mentions entries and the at-mentions decision record note line-range as landed with deferred items still deferred
- [ ] Parity tracker section 4 checks off `@dir/`, multi-ref, and missing-file block as landed; only line-range plus docs remain as this slice's closing items
- [ ] Verified by help-text assertion plus manual README and tracker parity check (no automated README test)
