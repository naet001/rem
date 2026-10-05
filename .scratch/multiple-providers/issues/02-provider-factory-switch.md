# 02: Provider factory with live switch preserving history

**What to build:** The Principal builds either the OpenAI-compatible leg or the Anthropic-native leg from the active provider entry, maps the unified low/medium/high effort per provider, and can switch providers mid-Session by rebuilding only the inner agent while preserving caller-owned history, the Gate, recorder, mode, and skills.

**Blocked by:** 01: Multi-profile config with env-only keys.

**Status:** ready-for-agent

- [ ] Branches at inner-agent build time on provider type: OpenAI-compatible leg keeps chat-completions semantics with explicit base URL and reasoning-effort level; Anthropic leg uses the native Messages client with optional base-URL override and a thinking budget tier plus required max-tokens
- [ ] Maps unified low/medium/high to increasing per-provider reasoning budgets through one effort table
- [ ] Rebuilds the inner agent on switch reusing the same Gate hook instance, recorder, mode handle, and discovered skills; preamble (static instructions + project instructions + skills index) is rebuilt identically
- [ ] Preserves caller-owned conversation history verbatim across switches; fail-closed keeping the old agent when the rebuild fails
- [ ] Tracks active provider, active model, and session-local effort on the agent for footer and dispatch use
- [ ] Covered by factory tests without live network: branch selection per provider type and effort mapping table outcomes
