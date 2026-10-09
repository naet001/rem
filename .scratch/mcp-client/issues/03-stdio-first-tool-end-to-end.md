# 03: stdio first-tool end to end

**What to build:** Spawn a local `command` MCP server over stdio and use its first tool inside a real session. After this ticket the Principal sees MCP tools in the preamble index, the Approver gets a risk-first Confirm modal showing `mcp__server__tool` plus server origin, and the transcript shows live Tool rows plus Trailer counts exactly like built-ins.

**Blocked by:** 01-shared-tool-server-handle-prefactor, 02-mcp-json-config-plus-gate-policy-arm.

**Status:** done

- [ ] `command` + `args` + `env` entry spawns, lists tools, registers one runtime-defined tool per `mcp__server__tool` on the shared handle; RunningService held for process lifetime
- [ ] Server `instructions` (if any) prepended to tool description; preamble index advertises MCP tools so the Principal reaches for them
- [ ] Dispatch records ToolEvent rows with arg preview through live stream, scrollback elision, and Done / Interrupted Trailer counts
- [ ] Approval modal is blocking FIFO, risk-first (origin + reason before args), with Allow-once / Allow-always / Deny; Deny replans inside the same Run
- [ ] Spawn / connect / list-tools failure logs to stderr, marks server failed with reason, session proceeds on remainder plus built-ins
- [ ] Tool-list change refreshes without polling or restart; mid-session drop retires that server's tools with a replannable tool error
- [ ] Mocked-peer bridge test covering naming + warn-and-continue + transcript events green
