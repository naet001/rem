# Configuration

`rem` reads exactly one config file. There is no walk-up search, no
per-directory override, and no precedence list.

```
$XDG_CONFIG_HOME/rem/config.toml      # Linux  (~/.config/rem/config.toml)
~/Library/Application Support/rem/config.toml   # macOS
%APPDATA%\rem\config.toml             # Windows
```

Resolved from `dirs::config_dir()` (`src/config.rs:57`). A missing or unreadable
file is fatal: `rem` prints `failed to read {path}`, hints at
`config.example.toml`, and exits 1. Copy the example and edit it:

```bash
mkdir -p ~/.config/rem
cp config.example.toml ~/.config/rem/config.toml
```

## Schema

Both `Config` and `ProviderConfig` are annotated
`#[serde(deny_unknown_fields)]`, so **a typo in a key name is a hard parse
error**, not a silently ignored line.

### Top level

| Key | Type | Required | Default |
|---|---|---|---|
| `active_provider` | string | yes | — |
| `providers` | table | yes | — |
| `effort` | string | no | `"medium"` |
| `theme` | string | no | unset (built-in palette) |

`active_provider` must name a key in `providers`; an unknown name fails when the
provider is resolved. `theme` names a file in `~/.config/rem/themes/` — see
[themes.md](themes.md).

### `providers.<name>`

| Key | Type | Required | Notes |
|---|---|---|---|
| `type` | string | yes | `openai-compatible` or `anthropic` |
| `key` | string | yes | `env:VAR` indirection only |
| `default_model` | string | yes | used at startup |
| `base_url` | string | no | see defaults below |

Only two provider kinds exist (`ProviderKind`, `src/config.rs:21`). When
`base_url` is omitted, an `anthropic` provider falls back to
`https://api.anthropic.com` (`src/config.rs:104`). An `openai-compatible`
provider has no default and must set `base_url` explicitly.

```toml
active_provider = "openai"
effort = "medium"

[providers.openai]
type = "openai-compatible"
base_url = "https://api.openai.com/v1"
key = "env:OPENAI_API_KEY"
default_model = "gpt-4o"

[providers.anthropic]
type = "anthropic"
key = "env:ANTHROPIC_API_KEY"
default_model = "claude-sonnet-4-5"
```

## API keys

`key` accepts only the form `env:VARNAME`. The literal secret never sits in the
file, so a config is safe to paste in an issue. Export the variables before
launching.

Three distinct errors, quoted from `resolve_key` (`src/config.rs:136`):

| Input | Error |
|---|---|
| `key = "sk-abc…"` | `provider "<name>" key must use env:VAR indirection, got a literal` |
| `key = "env:"` | `provider "<name>" key has empty env: reference` |
| `key = "env:FOO"`, `FOO` unset | `provider "<name>": env FOO not set` |

Keys are re-resolved on every `/model` and `/effort` switch, so a provider
becomes usable as soon as its variable is exported — no restart needed.

## Effort

`effort` is a single knob, stored as a plain string and normalized at use time
(`normalize_effort`, `src/agent.rs:577`): `low` and `high` pass through, and
**anything else — including `""` or `ultra` — becomes `medium`**. No config
error, no warning.

Each provider maps it differently:

- **openai-compatible** — sends `{"reasoning_effort": "..."}` as an extra body
  field.
- **anthropic** — sets a thinking budget and matching `max_tokens`:
  `low` → (1024, 4096), `medium` → (4096, 8192), `high` → (8192, 16384).

`/effort <level>` overrides for the session only and never writes the config
file.

## Legacy sections

`[api]` and `[model]` tables are rejected with a migration hint rather than
being ignored (`src/config.rs:78`). If you are upgrading from a pre-provider
config, move those values into a `[providers.*]` block.

## Related files

Everything `rem` stores lives beside the config file under
`~/.config/rem/`:

| Path | Purpose |
|---|---|
| `config.toml` | this file |
| `themes/<name>.toml` | color themes ([themes.md](themes.md)) |
| `skills/` | skill bundle search path ([README](../README.md#skills)) |
| `mcp.json` | MCP servers ([mcp.md](mcp.md)) |
| `mcp-tokens.json` | MCP OAuth tokens, mode 0600 |

Session history is **not** here: it lives in the platform data directory
(`~/.local/share/rem/sessions.db` on Linux), overridable with `REM_SESSIONS_DB`.
See [the sessions section](../README.md#sessions).

## Errors

Config failures are fatal at startup by design — `rem` will not run against a
half-understood configuration. The messages name the offending provider or key.
The one exception is MCP: bad `mcp.json` entries warn and are skipped, and
startup never fails because of MCP.
