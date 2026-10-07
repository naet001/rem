# 01: Shared ToolServerHandle prefactor

**What to build:** Move the AgentLoop's 11 built-in tools onto a shared ToolServerHandle so later MCP tools can join the same registry without touching the Gate / Approver / Run loop. After this ticket the session behaves exactly as before — same preamble, same Policy verdicts, same transcript Tool rows — but the registry is ready for runtime-defined tools.

**Blocked by:** None (can start immediately).

**Status:** done

- [ ] All 11 built-ins (Read / Mutate / Execute / Network classes) resolve through the shared handle at dispatch
- [ ] Existing Policy (Allow / Confirm / Deny), session rules, Plan / Yolo semantics unchanged
- [ ] Transcript Tool rows, live stream, Trailer counts unchanged
- [ ] `cargo build`, `cargo test`, `cargo fmt`, `cargo clippy -D warnings` green
