# Contributing

`rem` is a single-binary Rust project. This page covers how to build it, how it
is tested, and how a change actually reaches `main`.

## Requirements

Rust with edition 2024 support (rustc 1.85 or newer) and a reachable
OpenAI-compatible endpoint or Anthropic key. No other runtime dependencies: the
SQLite driver is `rusqlite` with the `bundled` feature, so there is no system
libsqlite to install.

```bash
git clone https://github.com/naet001/rem
cd rem
cargo build
cargo run                  # reads ~/.config/rem/config.toml
```

## Checks

CI runs all three on every push to `dev` and on every pull request. Run them
locally before pushing; a red check blocks the promote step.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo test` is **offline-green**: it passes with no network and no API keys.
Two live checks in `src/tools/web.rs` are marked `#[ignore]` and are run
explicitly:

```bash
cargo test -- --ignored    # needs DuckDuckGo + general internet
```

## Layout

One binary, no workspace. Tests live in `#[cfg(test)]` modules next to the code
they cover — there is no `tests/` directory.

| Area | Files |
|---|---|
| Entry, CLI parsing | `src/main.rs`, `src/headless.rs` |
| Agent loop, preamble, compaction wiring | `src/agent.rs` |
| Tools | `src/tools/*.rs` (one module per tool, shared diff + error type in `mod.rs`) |
| MCP client | `src/mcp.rs` |
| TUI | `src/tui.rs` (commands, submit path, transcript), `src/popup.rs`, `src/history.rs`, `src/markdown.rs` |
| Session state | `src/context.rs` (history, tool-result budget, checkpoints), `src/sessions.rs` (SQLite) |
| Config and theme | `src/config.rs`, `src/theme.rs` |
| Skills and refs | `src/skills.rs`, `src/context.rs`, `@` expansion in `src/tui.rs` |

Rough sizes: ~21k lines of Rust across 25 files, ~320 tests. `src/tui.rs` alone
holds over a third of them.

## Where behavior is defined

Do not trust prose when it disagrees with the code — these are the authorities,
in order:

1. **The source.** Constants and caps are named in code
   (`FANOUT_MAX_TASKS`, `TOOL_BUDGET`, `MAX_CHECKPOINTS`).
2. **`docs/glossary.md`** — the living source of truth for terminology and the
   transcript model.
3. **`docs/adr/`** — decisions on record, dated. Several describe behaviour that
   was later removed; the glossary header says which (the permission gate from
   ADR-0001/0002/0003/0011 no longer exists, and ADR-0014's `--mode`/`--allow-all`
   were never kept). Read an ADR for its reasoning, verify the current state in
   code.
4. **The README and the rest of `docs/`** — reference for users.

When you change behavior, update the README, the matching doc page, and open a
new ADR if the decision is non-obvious. ADR numbers in this repo are not unique
(there are two `0009`, `0010`, `0011`, `0012`, `0013`); the filename is the
identifier.

## Branch flow and releases

The intended flow, as encoded in the two workflow files:

```
feature work → dev ──tests pass──▶ promote (needs approval) ──▶ main ──▶ tag v*.*.* ──▶ GitHub Release
```

Two workflows:

- **`.github/workflows/dev.yml`** — `fmt`, `clippy`, and `test` on any push or PR
  to `dev`. If green, a `promote` job gated on the `production` environment
  waits for a human review; approving merges `dev` into `main`, bumps the patch
  tag (`v0.1.4` → `v0.1.5`), syncs `Cargo.toml` and `Cargo.lock` to match, and
  pushes both.
- **`.github/workflows/main.yml`** — on a push to `main` or a `v*.*.*` tag,
  builds release binaries for `x86_64-unknown-linux-gnu` and
  `aarch64-apple-darwin`. A tag additionally publishes a GitHub Release with
  those assets and generated notes.

Because the version is derived from the newest tag, `Cargo.toml`'s `version` is
managed by CI on `main` — do not hand-bump it in a `dev` PR.

The approval gate is one-time setup on the repo:
**Settings → Environments → New environment → `production` → Required reviewers →
add yourself.** Without it the promote job runs unattended.

Not yet exercised: `origin` currently holds only `dev`, with no `main` branch and
no tags, so the promote and release halves of this pipeline are untested. The
first promotion should be watched closely.

## Conventions

- Edition 2024, `cargo fmt` defaults; formatting is enforced in CI, so do not
  hand-align.
- `clippy` runs with `-D warnings` across all targets, test code included.
- Doc comments state *why* a constraint exists (a cap, an abort semantic, a
  warn-and-continue choice), not what the line does. Existing modules follow
  this — see `src/mcp.rs:1-3`.
- Errors shown to the model are its chance to correct course: keep them specific
  and name the offending input, as `edit` does when `oldText` is ambiguous.
- No hardcoded colors in the render path; every color comes from `&Theme`,
  passed into `history.rs`, `markdown.rs`, and `tui.rs`
  ([themes.md](themes.md)).
- Never write a secret into config, fixtures, or logs. Config enforces `env:`
  indirection for provider keys for exactly this reason.

## Before opening a PR

- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test` all clean.
- [ ] Behavior change covered by a test next to the code.
- [ ] README and the relevant `docs/*.md` updated — caps, defaults, and command
      lists go stale quickly.
- [ ] Non-obvious decision recorded as an ADR.
- [ ] User-visible TUI change checked in a real terminal, not only in tests.
      [`docs/popup-test-matrix.md`](popup-test-matrix.md) is the manual matrix
      for popup and keymap behavior.

## License

By contributing, you agree that your contributions are licensed under the MIT
License, the same terms as the rest of the repository.
