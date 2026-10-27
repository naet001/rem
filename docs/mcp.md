# MCP servers

`rem` is an MCP client. Servers declared in one JSON file contribute tools that
the agent can call exactly like its built-ins — same immediate execution, no
approval step. Design record:
[ADR-0013](adr/0013-mcp-client.md).

## File

```
~/.config/rem/mcp.json
```

Same platform config directory as `config.toml` (`default_path`,
`src/mcp.rs:12`). The file is optional: **missing = zero servers, no error, no
warning**. Startup never fails because of MCP — every failure mode degrades to a
warning printed on stderr and the session continues.

## Shape

Claude-compatible `mcpServers` object, keyed by server name. Entries are parsed
in name-sorted order (`src/mcp.rs:1652`).

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/srv/data"],
      "env": { "LOG_LEVEL": "warn" }
    },
    "github": {
      "url": "https://api.githubcopilot/mcp",
      "headers": { "Authorization": "Bearer ${GITHUB_TOKEN}" },
      "auth": true
    }
  }
}
```

| Field | Transport | Required | Notes |
|---|---|---|---|
| `command` | stdio | yes, when no `url` | program to spawn |
| `args` | stdio | no | string array; non-string items are dropped silently |
| `env` | stdio | no | string map passed to the child |
| `url` | HTTP | yes, to select HTTP | presence selects streamable-HTTP |
| `headers` | HTTP | no | string map sent on every request |
| `auth` | HTTP | no | `true` opts into OAuth; default `false` |

Two transports are supported: **stdio** (`TokioChildProcess` wrapped by
`process-wrap`, `src/mcp.rs:969`) and **streamable HTTP**
(`StreamableHttpClientTransport`, `src/mcp.rs:1044`). SSE is not supported.

There is no `disabled` or `type` field. `url` wins if both `url` and `command`
are present. An entry that has neither, or whose `command`/`url` is empty after
variable expansion, is skipped with `mcp: skipping invalid entry "<name>"`.

## Environment expansion

`${VAR}` and `${VAR:-default}` are expanded inside `command`, every `args`
element, every `env` value, `url`, and every `headers` value
(`expand_env`, `src/mcp.rs:1615`):

- set variable → its value
- unset with `:-default` → the default
- unset without a default → **empty string** (which usually makes the entry
  invalid and therefore skipped)
- unterminated `${` → copied through literally, no error

This is the only place `rem` accepts a secret for MCP; the value never needs to
be written down.

## Tool naming and registration

Server tools are registered as Rig `DynamicTool`s on the shared tool server
handle, so `/mcp on` and `/mcp off` take effect mid-session
(`src/mcp.rs:605`):

```
mcp__<sanitized-server>__<tool>
```

Server names are sanitized to `[A-Za-z0-9_-]`, every other character replaced
with `_` (`src/mcp.rs:1749`). The server's `instructions` string is prepended to
each of its tool descriptions, and an inventory index is appended to the system
preamble so the model knows what is available before it calls anything
(`preamble_section`, `src/mcp.rs:313`).

A server that connects and lists zero tools is an error:
`mcp "<name>": server listed zero tools`.

Servers can push `list_changed`, which re-syncs that server's tools — skipped
for servers you have turned off this session.

## `/mcp`

Local only; none of these start a model turn. Toggling is refused while a turn
is running.

| Command | Effect |
|---|---|
| `/mcp` | list servers with status and tool counts |
| `/mcp on <name>` | connect and register tools this session |
| `/mcp off <name>` | drop the connection and unregister `mcp__<name>__*` |
| `/mcp login <name>` | OAuth browser flow (HTTP servers with `"auth": true`) |
| `/mcp logout <name>` | delete the stored token and mark the server needs-auth |

Bare `/mcp` prints one indented line per server — state, tool count, then a
reason or hint (`src/tui.rs:3075`):

```
mcp servers:
  github — connected (12 tools)
  linear — needs-auth (0 tools) — run /mcp login <name>
  legacy — failed (0 tools) — server listed zero tools
  notes — connected (3 tools) [disabled]
```

`connected` carries no suffix, `failed` appends the warn-and-continue reason,
and `needs-auth` appends the literal hint `— run /mcp login <name>`. The tool
count is singular at one. With no file present you get
`no MCP servers configured (see ~/.config/rem/mcp.json)`. Anything other than
the four subcommands prints
`usage: /mcp [on <name> | off <name> | login <name> | logout <name>]`.

## OAuth

`/mcp login <name>` works only for HTTP servers; a stdio server gets an explicit
error. The flow (`src/mcp.rs:1192`):

1. Discover authorization-server metadata.
2. Build an `AuthorizationSession` with dynamic client registration and
   resource `"rem"`.
3. Bind an ephemeral loopback listener on `127.0.0.1:0` at `/callback`.
4. Print the URL and try to open a browser:
   `mcp: open this URL to log in <name>:\n<url>`
5. Exchange the returned code for tokens, store them, reconnect, and register
   tools → `mcp "<name>" logged in (<n> tools)`.

The callback waits up to **300 seconds**, with a 30-second limit on reading the
callback body. `/mcp logout <name>` reports `mcp "<name>" logged out`.

Tokens live in `~/.config/rem/mcp-tokens.json` (`src/mcp.rs:41`) — a JSON map of
server name to `{ access_token, refresh_token?, scopes[] }`, written atomically
via temp file + rename and `chmod 0600` on Unix. If the config directory is
unavailable, it falls back to `mcp-tokens.json` in the working directory.

### Expired and revoked tokens

An auth-shaped failure is matched case-insensitively against `401`,
`unauthorized`, `invalid_token`, `insufficient_scope`, and
`authorization required`. On a match `rem` refreshes once and retries once; if
that still fails the server is marked needs-auth and the tool error names the
server plus a `/mcp login <name>` hint (`src/mcp.rs:624`).

## Scope and limits

- No cap on the number of servers or tools.
- Per-call timeout is not set by `rem`; it inherits Rig's
  `DEFAULT_MCP_TOOL_TIMEOUT` of 300 seconds.
- **Subagents do not see MCP tools.** `task` children are built from a fixed
  chain of the 12 built-ins (`build_child_inner`, `src/agent.rs:864`). Only the
  principal agent uses MCP.
- MCP tools are unprivileged in the same sense as built-ins: no allowlist, no
  confirmation, no sandbox. A `write`-shaped tool on a server writes wherever
  your user can write.

## See also

- [ADR-0013 — MCP client](adr/0013-mcp-client.md)
- [tools.md](tools.md) — the 13 built-ins
- [configuration.md](configuration.md) — the config file beside `mcp.json`
