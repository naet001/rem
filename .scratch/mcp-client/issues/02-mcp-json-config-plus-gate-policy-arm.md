# 02: mcp.json config plus Gate Policy arm

**What to build:** Declare MCP servers in a separate `mcp.json` file in Claude `mcpServers` shape and gate every future `mcp__*` call through the existing Gate / Policy / Approver Run loop. After this ticket a user can write stdio and streamable-HTTP entries with env expansion, invalid entries warn-and-skip, and any `mcp__server__tool` name classifies Confirm (Yolo Allows, Plan Denies) with High risk — even before any server is actually spawned.

**Blocked by:** None (can start immediately).

**Status:** done

- [ ] Missing `mcp.json` means zero MCP servers with no error and session starts on built-ins
- [ ] `url` key selects streamable-HTTP, `command` key selects stdio; `${VAR}` and `${VAR:-default}` expand in command, args, env, url, headers
- [ ] Invalid entry warns and skips that server; startup never fails because of MCP
- [ ] Server names sanitize to `[A-Za-z0-9_-]` and tools namespace as `mcp__server__tool`; bare upstream names never exposed
- [ ] Gate Policy: `mcp__*` → Confirm by default, Yolo Allow, Plan Deny with model-visible feedback; annihilator Deny floor does not apply
- [ ] Risk High with Third-party reason, compact-args preview pinned, session `always allow` keys per exact `mcp__server__tool:args`, audit trail entries on Approve / Deny
- [ ] Pure unit tests for load + expand + sanitize + Policy arm green alongside existing classify tests
