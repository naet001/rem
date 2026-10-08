//! Process-level sandbox shim for shell commands.
//!
//! Not an OS sandbox (no Seatbelt/bubblewrap): cwd jail + env scrub +
//! path denylist + network-verb gating. Documented deviation from Claude
//! (see ADR-0015).
