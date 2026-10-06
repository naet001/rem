# 04: Model and effort dispatch with footer

**What to build:** A user can list and switch models mid-Session with a single slash command (optional provider prefix) and adjust reasoning effort session-locally, with both commands discoverable in the menu and help, safe while a Run is busy, and reflected in the footer.

**Blocked by:** 02: Provider factory with live switch preserving history.

**Status:** ready-for-agent

- [ ] Bare `/model` lists all `provider:model` combos from configuration
- [ ] `/model <name>` switches models within the active provider; `/model <provider>:<name>` switches providers and models in one step
- [ ] Bare `/effort` shows the current level; `/effort low|medium|high` applies live to the active provider without rewriting the config file
- [ ] Both commands are refused while a Run is busy; unknown model or level names error without touching the live agent
- [ ] Every successful switch enqueues a system notice in the transcript and updates the footer to show `provider:model` plus effort
- [ ] Slash menu and help list the new commands; conversation history is preserved across switches
- [ ] Covered by dispatch tests through the AgentLoop test double: list, same-provider switch, cross-provider switch, unknown-name safety, busy refusal, footer state
