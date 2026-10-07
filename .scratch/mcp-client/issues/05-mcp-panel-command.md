# 05: mcp panel command

**What to build:** Inspect and control MCP servers from inside the TUI via `/mcp`. After this ticket the user can diagnose connected / failed-plus-reason / needs-auth states with tool counts and quiet a noisy server for the session without editing config or restarting.

**Blocked by:** 03-stdio-first-tool-end-to-end.

**Status:** done

- [ ] `/mcp` lists servers as connected / failed plus reason / needs-auth with per-server tool counts, reusing popup shell and slash-menu patterns
- [ ] Per-server session-scope enable / disable toggle takes effect immediately without restart or config edit
- [ ] `/help` and slash menu document `/mcp`, matching `/skills` / `/resume` / `/theme` discovery
- [ ] StubAgent slash-pattern tests cover list / toggle / empty-states without live servers
