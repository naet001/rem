# 04: README + help parity for Verdicts, Modes, and Limits

**What to build:** One consistent new-user story across README and `--help`. A new user reaches a first Run in minutes via config sample + first run + quit + resume hint, understands Verdicts (Allow / Confirm / Deny) with Tool classes plus annihilator Deny floor, can choose a Permission mode (plan / manual / auto / edit / yolo + yolo safety floor), sees honest Limits (heuristic-not-sandbox Execute classification, no secrets redaction, local single-user Approver), and can reproduce the dev gate locally. ADRs + glossary remain linked source of truth, not duplicated. Help text and README agree.

**Blocked by:** 02-headless-help-version, 03-dual-license.

**Status:** ready-for-agent

- [ ] README quickstart shows config creation + first run + quit + resume hint, verified against actual `--help`
- [ ] README + help agree on flags and slash-command pointers (no drift)
- [ ] README documents Verdicts table + Tool classes + annihilator Deny floor using glossary vocabulary
- [ ] README documents Permission modes table + yolo safety floor
- [ ] README Limits block states heuristic-not-sandbox, no-redaction, local single-user Approver scope
- [ ] README development section documents exact green gate commands (build, fmt check, clippy -D warnings, default offline test + opt-in live command)
- [ ] ADRs/glossary linked, not copy-pasted; frozen contracts (choke-point, tiers, interrupt-resume, Aster panes, keymap, slash-menu, busy verbs, project instructions, sessions, modes/popups, at-mentions, popup nav, skills) unchanged
