# 06: full OAuth

**What to build:** Log in to OAuth-only hosted MCP servers with full MCP OAuth 2.0. After this ticket a missing or expired token prints the auth URL and attempts `open` / `xdg-open`, `/mcp login <name>` / `/mcp logout <name>` re-auths from inside the TUI, and a mid-session 401 refreshes once, retries once, then marks the server needs-auth with a model-visible error naming the server.

**Blocked by:** 04-streamable-http-plus-static-headers, 05-mcp-panel-command.

**Status:** done

- [ ] Dynamic client registration plus localhost-callback browser flow; startup prints auth URL and attempts `open` / `xdg-open` when token missing or expired (printable URL covers SSH / headless)
- [ ] Tokens stored 0600, never logged; `/mcp login <name>` / `/mcp logout <name>` drive re-auth from inside the TUI and update panel status
- [ ] Static-header servers keep working unchanged alongside OAuth servers
- [ ] Mid-session 401 → refresh once → retry once → mark needs-auth; Principal receives tool error naming the server so it can ask the Approver to log in instead of hanging
- [ ] Headless non-interactive OAuth stays out of scope with a clear error
