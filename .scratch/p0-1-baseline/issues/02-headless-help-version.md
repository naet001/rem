# 02: Headless-safe --help / --version before Session / Principal / TUI init

**What to build:** A new evaluator with no config file and no TTY runs `rem --help` / `rem --version` as a subprocess and gets usage/version text on stdout with exit 0, creating no Session row and never booting the Principal or fullscreen TUI. Unknown flags exit non-zero on stderr pointing at help. Existing Session resume-by-id behavior is unchanged.

**Blocked by:** 01-verify-clean-tree.

**Status:** ready-for-agent

- [ ] `--help` prints usage (flags incl. resume-by-id, slash-command pointers, Verdicts/modes summary, config location) to stdout, exits 0, with no config file present
- [ ] `--help` works with no TTY and performs no filesystem writes / allocates no Session row / never initializes TUI
- [ ] `--version` prints packaged version string to stdout, exits 0, with no side effects
- [ ] Unknown flags print to stderr, exit non-zero, point at help
- [ ] Argument dispatch runs before config load, sessions-db open, Principal construction, and TUI init
- [ ] `--resume <id>` restarts anywhere as before; unknown ids remain fatal with clear error; Picker/ Fork/ Title/ Autosave semantics unchanged
