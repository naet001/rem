//! Heuristic auto-mode classifier: blocked/allowed lists as data.
//!
//! Ports Claude's documented blocked-by-default / allowed-by-default
//! categories to pattern checks over (tool, args, workdir, remotes).
//! No LLM calls: fast, offline, fails safe to Confirm.
