# 04: streamable HTTP plus static headers

**What to build:** Connect to remote MCP servers over streamable HTTP with static headers, reusing the same Gate / transcript path proven by stdio. After this ticket a `url` entry with Bearer headers reaches a hosted server and its tools behave identically to local ones from the Approver's perspective.

**Blocked by:** 03-stdio-first-tool-end-to-end.

**Status:** done

- [ ] `url` entries connect over streamable HTTP via reqwest with rustls; `headers` support `${VAR}` expansion
- [ ] Remote tools register as `mcp__server__tool`, appear in preamble index, dispatch through the shared handle with same Confirm / Yolo / Plan Policy and High risk
- [ ] Connect / list-tools failure warn-and-continues with stderr log plus failed-with-reason status; session proceeds
- [ ] Mid-session drop and list-changed refresh behave as in stdio slice (retire with replannable error, event-driven refresh)
- [ ] No legacy SSE client dialect; SSE carried inside streamable HTTP works, pure SSE-only servers get a clear error
