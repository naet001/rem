# 01: Verify clean tree + quarantine live network checks

**What to build:** A trustworthy green baseline. A contributor on a clean checkout gets a successful build plus passing formatter, linter (warnings denied), and default test run with no network access. The single live-network web search check is quarantined out of the default run behind explicit opt-in, so CI red can only mean real regression. No Gate / Policy / Approver behavior change.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Codebase-wide conflict-marker sweep (agent-loop orchestrator, startup entrypoint, terminal UI modules) returns empty
- [ ] Clean-checkout build succeeds
- [ ] Formatter check passes
- [ ] Linter with warnings denied across all targets passes
- [ ] Default test run passes fully offline (167+ passing, 0 failures requiring network)
- [ ] Live-network web checks are ignore-by-default with documented explicit opt-in command
- [ ] No change to classify tiers, Mutate fast-path, Execute safe-list, Network Confirm default, annihilator Deny floor, unknown-tool fail-closed, or Interrupt / Resume semantics
