# Claude Code Modes & Permissions — Research Notes

Date: 2026-10-08
Question: how do Claude Code's permission modes and permissions work?

Primary sources (all first-party, read 2026-10-08):
- Permissions: https://docs.anthropic.com/en/docs/claude-code/permissions
- Permission modes: https://code.claude.com/docs/en/permission-modes
- Auto-mode config: https://code.claude.com/docs/en/auto-mode-config
- Settings & precedence: https://docs.anthropic.com/en/docs/claude-code/settings
- Sandboxing: https://code.claude.com/docs/en/sandboxing
- Common workflows (plan mode entry): https://code.claude.com/docs/en/common-workflows.md

> Terminology note: what the CLI/VS Code UI calls **"Manual"** has config value
> `"default"`. `manual` is accepted as an alias on the CLI/flags
> (source: permission-modes page).

## 1. Two layers: rules + modes

- **Permission rules** (`permissions.allow / ask / deny` in settings) decide per-tool-call:
  allow = run without prompt, ask = prompt, deny = block.
  Evaluated in fixed order **deny → ask → allow**; first match in that order wins.
  Specificity does NOT change the order — a broad `Bash(aws *)` deny beats a narrow
  `Bash(aws s3 ls)` allow; a matching ask beats a matching allow.
  (source: permissions page, "Manage permissions").
- **Permission modes** set the baseline for everything not covered by a rule:
  which tool calls prompt vs run vs get reviewed by the auto-mode classifier.
  Modes never override deny rules — deny blocks in **every** mode including
  `bypassPermissions`. Allow rules have no effect in `bypassPermissions`
  (nothing left to pre-approve). (source: permission-modes, "Available modes").
- Bare-name deny (`"deny": ["Bash"]`) **removes the tool from Claude's context**
  so it never even tries to call it; scoped deny (`Bash(rm *)`) leaves the tool
  visible and blocks matching calls. Exception: `EndConversation` can't be removed
  while any other tool remains, and ask never prompts for it.
  (source: permissions page).
- The permission prompt offers Allow-once / "don't ask again" / session-scope choices.
  "Don't ask again" for Bash/WebFetch persists to `.claude/settings.local.json` at the
  **git repo root** (resolved through worktrees to main checkout), applying repo-wide
  including subdirectories/worktrees (since v2.1.211; before that: starting dir only).
  File-edit approvals last until session end, not persisted. Some prompts are one-shot
  only (no "don't ask again") when the prompt can't show everything the rule would cover.
  (source: permissions page, "Permission system").
- Comment-on-prompt: on Bash/PowerShell/file/MCP prompts, move to Yes/No and press
  `Tab` for a comment field. `Yes` runs the action then delivers the comment after the
  result; `No` sends it as the denial reason and Claude continues (bare `No` on a main-
  conversation prompt stops the turn). WebFetch/browser prompts have no comment field.
  (source: permissions page).

## 2. Rule syntax `Tool` / `Tool(specifier)`

(source: permissions page, "Permission rule syntax" and subsections)

- Format is `Tool` or `Tool(specifier)`; parens inside the specifier are literal.
- `Bash(*)` ≡ `Bash` (all commands). A rule with no `*` is one exact command;
  `*` matches any text including spaces. Trailing `" *"` (space-star) also matches the
  bare command (`Bash(ls *)` matches `ls`); `:*` suffix is an equivalent trailing
  wildcard (`Bash(ls:*)`); `:*` is only special at the end.
- Bash rules match the whole command text **after** splitting compound commands and
  stripping wrappers:
  - **Compounds**: deny/ask fire if ANY subcommand matches, including inside subshells,
    `$(...)`, `for` bodies (`cd /tmp && git clean -f` still matches `Bash(git clean *)`).
    "Don't ask again" on a compound saves one rule per subcommand needing approval
    (up to 5), not one rule for the whole string. `&&`/`||` with nothing after is
    treated unparseable → allow-prefix rules don't match it.
  - **Stripped wrappers** (built-in, not configurable): `timeout time nice nohup stdbuf`,
    `command`, `builtin`, zsh `noglob`, safe env assignments (`NODE_ENV=test npm test`),
    bare `xargs` (no flags). NOT stripped: `direnv/mise/devbox/npx/docker exec`,
    `watch/setsid/ionice/flock`, `find -exec/-delete` — write exact-match rules for those.
  - A Bash rule does NOT match the same program invoked another way
    (`git push` rule misses `git -C . push`); it is not a security boundary. Use
    sandboxing for enforcement that doesn't depend on command text, or a PreToolUse hook
    to inspect full text.
- **Built-in read-only Bash set** runs without prompting in every mode (except as
  `blockReadsOutsideWorkingDirectories` changes for outside paths): `ls cat echo pwd
  head tail grep find wc which diff stat du cd` + read-only `git` forms. Not configurable;
  add ask/deny to force a prompt. Still prompts when: unquoted glob on a write/exec-capable
  command (`find/sort/sed/git` + `*`), `docker -H/--context`, `file -m/-f`, Windows UNC
  path, writes to special shell vars (`PATH`, `IFS`), unparseable or >10k chars,
  `cd X && git …` into a different dir (hooks risk), `cd …` + ambiguous redirect target
  (`/dev/null`-only is fine).
- **Redirections**: `> >> 2>` targets are checked as if Claude wrote the file
  (Edit allow/deny + protected paths + working dirs); `<` input targets checked against
  Read rules + working dirs (v2.1.257+); `tee` write targets checked too (v2.1.269+).
  `/dev/null`, fd-dups (`2>&1`), here-docs/strings are unchecked. `~`/glob targets need approval.
- **PowerShell** rules mirror Bash; aliases canonicalized (`gci/ls/dir` ≡ `Get-ChildItem`),
  case-insensitive, AST-parsed per subcommand (`|`/`;`/`&&`/`||` split; all must match).
- **Read/Edit file rules** use gitignore syntax; only `Edit(path)` and `Read(path)` are
  consulted (a `Write(...)`/`NotebookEdit(...)`/`MultiEdit(...)`/`Glob(...)` path rule is
  accepted but never consulted + startup warning; bare `Write` with no path still matches
  at tool level). `Read` deny also blocks Edit/Write on that path (edits v2.1.208+, writes
  v2.1.228+) and is best-effort applied to Grep/Glob/`@file`/IDE context. NotebookEdit
  needs its own Edit deny.
  - Anchors: `/path` anchors at the settings-source dir (`/permissions` tab saves anchor
    at primary working dir, file lives at repo root); `~/` = home; `//` = absolute;
    bare names match at any depth (`Read(.env)` ≡ `Read(**/.env)`).
  - Depth quirk for single-segment `src/**`: **allow** = only `<cwd>/src`; **deny/ask** =
    any-depth `src`. Use `**/src/**` to mean any-depth in allow. `!` negations carve out
    only earlier rules in the SAME file and can't reopen a wholly-blocked dir or reach
    `/`/`~/`/`//`-anchored rules.
- **Symlinks**: check covers requested path AND resolved target. Allow needs BOTH to match;
  deny fires if EITHER matches. Requested-inside/symlinked-outside fails allow. Writes
  through symlinks resolving outside working dirs or onto protected paths lose auto-approval
  (prompt instead of classifier). Unresolvable/looping paths are refused; TOCTOU re-checked
  at open.
- **WebFetch** rules are `WebFetch(domain:host)`, case-insensitive, trailing-dot-insensitive.
  `*.example.com` = any-depth subdomains (not bare); mid-pattern `*` stays within one label
  (`example.*` ↛ `example.evil.com`). Bare `WebFetch` deny/ask ≠ `domain:*`: only the
  `domain:` form feeds the sandbox allow/deny lists; only `domain:` rules gate claude.ai
  artifact reads.
- **MCP**: `mcp__server` (all its tools), `mcp__server__tool`; in settings, `mcp__*` rules
  with parens are skipped with warning (use `--disallowedTools` for param matching).
  Allow-globs require literal `mcp__<server>__` prefix; unanchored `"*"/"B*"/"mcp__*"`
  allow-globs are skipped. Org `ask`-pinned connector tools prompt in every mode incl.
  auto/bypass (dontAsk denies instead). Cowork `mcp__workspace__bash/web_fetch` inherit
  Bash/WebFetch *deny* but never *allow*.
- **Agent (subagents)**: `Agent(Name)`; **Cd**: `Cd` rules gate only the human-run `/cd`
  command (bare `Cd` deny disables it; any allow switches to allowlist; `*`=1 segment,
  `**`=cross-segment, trailing `/**` includes root).
- **Param matching** (`Tool(param:value)`, deny/ask only): top-level scalar fields only,
  one param per rule, `*` wildcard, omitted-param never matches, compared pre-normalization,
  content fields (`command`, `file_path`, `path`, `url`, …) rejected with startup warning.

## 3. Permission modes

(source: permission-modes page; workflows page for plan entry)

| Mode (config value) | Baseline behavior |
|---|---|
| Manual (`default`) | Stop-and-ask before most edits, shell commands, network. Status `⏸ manual mode on`. |
| `acceptEdits` | Auto-approve file create/edit **in working dir(s)** + `mkdir touch rm rmdir mv cp sed` there (safe-env/wrapper prefixes OK); PowerShell `Set/Add/Clear-Content`, `Remove-Item` (+aliases) same scope. Everything else = normal prompt flow. Quoted-arg edge (`"It's done"` positionally) still prompts — use `-Value`. Status `⏵⏵ accept edits on`. |
| `plan` | Research + propose, no edits until plan approved. Shell during planning: bypass-available interactive terminal → neither classifier nor prompt; else if auto available && `useAutoModeDuringPlan` (default on) → classifier reviews non-critical-path commands; else → prompt for non-read-only (sandbox auto-allow does NOT apply). Approve via "Yes, and use auto mode" / "Yes, manually approve edits" / "No, keep planning"; `Ctrl+G` edits plan in `$EDITOR`; `showClearContextOnPlanAccept` adds accept+clear-context option. Status `⏸ plan mode on`. Enter via `Shift+Tab` or `/plan` prefix or `--permission-mode plan`. |
| `auto` | Classifier model reviews actions instead of you (details §4). Explicit ask rules still prompt. Status `⏵⏵ auto mode on`. Since v2.1.283 the built-in default for interactive terminal + VS Code on all plans/providers (before: Pro/Max/Team only); HIPAA-orgs stay Manual default. Requires Opus 4.6+/Sonnet 4.6+/Haiku 5.5+/Fable (Anthropic API); Sonnet 5+/Opus 4.7+/Haiku 5.5+/Fable on Bedrock/Vertex/Foundry/gateway. Falls back to Manual when unavailable (flag, `disableAutoMode`, unsupported model, server-side off). |
| `dontAsk` | Auto-deny anything that would prompt (reads + allow/hook-approved still run). AskUserQuestion, org-pinned-ask connectors, `requiresUserInteraction` MCP tools denied. Status `⏵⏵ don't ask on`. For CI/restricted envs. Ignored by cloud sessions. |
| `bypassPermissions` | No prompts/checks; tool calls execute immediately **including protected-path writes**. Still prompts/blocks: explicit ask rules, critical-path `rm/rmdir`, `requiresUserInteraction`, org-pinned-ask connectors, cross-session-messaging safeguards, `blockReadsOutsideWorkingDirectories` reads, unsandboxed-retry approval, public-artifact reads. Opt-in at launch only (`--permission-mode bypassPermissions` / `--dangerously-skip-permissions` / `defaultMode` in user/`--settings`/managed) → then appears in `Shift+Tab` cycle; `--allow-dangerously-skip-permissions` adds to cycle without activating. First interactive use shows accept-responsibility dialog (sets `skipDangerousModePermissionPrompt`). Refused as root/sudo (unless sandboxed) and under `--restricted`. Cloud sessions ignore it. |

- **Cycling**: `Shift+Tab` cycles `auto → default → acceptEdits → plan` (+ enabled optionals
  after `plan`: `bypassPermissions` first, `auto` last). VS Code: mode indicator w/ same set
  (`initialPermissionMode` pins default; last-picked Manual/Edit/Auto remembered; Plan/Bypass
  are per-conversation). Desktop: mode selector (Bypass needs settings toggle / org policy).
  Web/mobile: cloud = Accept-edits/Plan/Auto; Remote-Control = local session's mode.
- **Start mode**: flag > `permissions.defaultMode` > built-in default. `auto`/`bypassPermissions`
  do NOT take effect from project `.claude/settings.json` or `.local.json`
  (`auto` → built-in default instead; `bypassPermissions` → Manual); set them in user/managed
  settings or per-session flag. (`bypassPermissions`-from-file lock lifted pre-v2.1.257.)
  VS Code never reads project files for start mode. Resume keeps prior mode (see sessions doc).
- **Kill switches**: `permissions.disableAutoMode: "disable"` / 
  `permissions.disableBypassPermissionsMode: "disable"` (any scope incl. self-lock; most
  useful in managed settings).
- **Never auto-approved in ANY mode** (incl. bypass): explicit ask matches, org-pinned-ask
  connectors, `AskUserQuestion` + `requiresUserInteraction` MCP tools, critical-path
  `rm/rmdir` (no allow/hook override), cross-session safeguards, `blockReads…` outside reads.
  (source: "Actions no mode auto-approves").

## 4. Auto mode classifier

(source: permission-modes "Eliminate prompts…" + auto-mode-config page)

- A **separate classifier model** reviews actions pre-execution; blocks escalation beyond
  the request, unrecognized infra, or prompt-injection-driven actions. Also reviews
  inter-agent `SendMessage`. Nudges Claude to not stop for clarifying questions (Proactive
  output style is the stronger non-prompting lever).
- **Server-side review**: on direct Anthropic API (terminal v2.1.271+ Pro/Max/Team, v2.1.278+
  Ent/API; `-p`/SDK/VS Code/desktop v2.1.281+; no-flag-fetch sessions v2.1.282+), cloud
  providers/gateways (v2.1.278+), apps-gateway (v2.1.280+) the server may check as part of
  model requests instead of local classifier calls. No-results → fall back to local classifier;
  no-verdict-for-action → **deny** (10 no-verdicts in a row stops the turn).
  `CLAUDE_CODE_AUTO_MODE_SERVER=0` forces local classifier.
- **Blocked by default** (non-exhaustive; `claude auto-mode defaults` prints full JSON):
  `curl|bash`, exfiltration to external endpoints, prod deploys/migrations, mass cloud deletion,
  IAM/repo grants, shared-infra edits, destroying pre-session files, force-push, secrets-in-
  commit/push/CI-secret-widening (any branch; pre-v2.1.211: default branch only), `reset --hard`/
  `checkout -- .`/`restore .`/`clean -fd`/`stash drop|clear`, `commit --amend` off-session-HEAD
  or pushed-HEAD (v2.1.198+; pure `-m` reword of own unpushed commit OK), `terraform/pulumi/cdk/
  terragrunt destroy` (+ destroy-plans), secret-manager/DNS/TLS writes, unapproved-own-PR
  merge / self-approve / CI-disable, automation-trigger comments (`atlantis apply`, `/deploy`),
  prod flag flips, protected-IaC applies, over-broad k8s writes (label-selector/`--all`,
  DaemonSets/admission webhooks), shells/port-forwards into sensitive remotes, public tunnels/
  reverse shells, live-credential printing, sensitive-location egress, public-registry bypass
  around internal mirror, `--insecure`-style guard disarms, nested agent loops with approvals/
  sandbox off, risky Chrome-computer-use off-origin sends, `/tmp` wildcard/age deletes,
  outbound sensitive details (PR/issue/commit text, paths, API PII, infra IDs — API PII needs
  explicit recipient+detail auth regardless of repo visibility, v2.1.203+), self-driving own
  tmux pane, deleting/weakening security tests, stateful-resource teardown Claude didn't create,
  base-URL/proxy/registry repoints, `git remote add/set-url` (unless user named it),
  pushes/PRs to other repos/orgs or public repos carrying secrets/PII/confidential material,
  sensitive-store/file content into commits/pushes/pastes/publishes without naming source+dest
  (v2.1.203+), transcript `.jsonl` writes (v2.1.205+), `rm -rf "$UNASSIGNED_VAR"`-style deletes
  (v2.1.205+; name the path or re-run literal to clear), IMDS/service-account creds + off-host
  routes + sibling-container scans (v2.1.257+; declare in Host-containment entry to allow),
  content-bearing public-paste links without naming the service (v2.1.261+).
- **Allowed by default**: in-workdir file ops, lockfile/manifest installs, `.env`→matching-API
  credential use, read-only HTTP, pushes to **any branch of the working repo** incl. default
  (deploy-named non-default branches like `production`/`gh-pages` judged on merits; content
  rules + deny rules + branch protection still apply), deleting Claude's own session jobs,
  writing/reviewing security code as the task, same-session agent messages, listed trusted
  domains/buckets/services (data-flow only), Chrome nav to trusted/internal/localhost/user-named URLs.
- **Trust config** (`autoMode.environment`, prose not regex; `"$defaults"` splices built-ins;
  read from user/managed/`--settings` — NEVER project files; additive across scopes so a
  dev `allow` can soften an org `soft_deny`): org context, source-control orgs, cloud buckets,
  internal domains/services/registry, sensitive-data locations+audiences, sensitive remotes,
  protected IaC scopes, network/host-containment posture. `/auto-mode-setup` drafts it from
  CLAUDE.md/README/config/remotes + recent commands (+opt shell-history/sibling-repos);
  `claude auto-mode config|defaults|critique|reset` inspects/validates/clears.
- **Rule overrides**: `hard_deny` (unconditional) > `soft_deny` (user-intent/`allow` can clear)
  > `allow` (soft exceptions) > explicit in-conversation approval for the exact dangerous
  specific (verb alone insufficient; one action unless granted standing; never clears
  hard_deny). In-conversation *boundaries* ("don't push") block until lifted; lost on
  compaction — use deny rules for guarantees. Narrow `Bash(...)` allow rules still resolve
  pre-classifier (destructive args can slip through); `autoMode.classifyAllShell: true`
  suspends ALL shell allow rules in auto mode (except critical-path checks) at latency cost.
- **Failure/deny UX**: blocked action → notice + `/permissions` **Recently denied** tab
  (`r` = retry-with-approval; no-verdict denials not listed — capture via `PermissionDenied`
  hook); 3-in-a-row or 20-total blocks → auto pauses, prompting resumes (non-`-p`;
  headless just skips and continues); mode switch mid-check discards the verdict.
- **First outside-workdir read**: with `blockReadsOutsideWorkingDirectories` off, first
  Read/Grep/Glob outside workdirs prompts allow-outside-once vs block-forever (writes the
  block setting to user settings) vs ask-next-time variants. Non-interactive/background:
  no prompt, reads run as before.
- **Checkpoints**: to force human gates inside auto (e.g. push/PR), add `permissions.ask`
  (`Bash(git push *)`, `Bash(gh pr create *)` — note `git -C/-c` bypass, use PreToolUse for
  airtight); edit classifier rules in `/permissions` **Auto mode** tab (v2.1.246+).

## 5. Protected paths & critical paths

(source: permission-modes "Protected paths" / "Critical paths")

- **Protected paths** (never auto-approved except bypass / bypass-available plan sessions;
  allow rules can't pre-approve — check runs before allow evaluation; `--restricted`
  sessions route them to deny): `.git .config/git .vscode .idea .husky .cargo
  .devcontainer .yarn .mvn`, `.claude` (minus worktrees/session plans/job-tmp/auto+agent
  memory MDs, `--plugin-dir` dirs), dotfiles (`.gitconfig .gitmodules` shells `.envrc`
  `.npmrc/.yarnrc/pnpm/bun` bazel pre-commit gradle/maven `.devcontainer.json .ripgreprc
  pyrightconfig .mcp.json .claude.json`). Symlink-resolved writes that land protected
  prompt (or deny under `--restricted`) even when the requested spelling isn't. Project
  `~/.claude` prompts offer session-scoped "allow .claude edits for this session".
- **Critical paths** (`rm/rmdir` circuit breaker; no allow/hook-`"allow"` overrides; deny
  still wins; outcome per-mode table): fs root, its top-level children, `$HOME`, drive
  roots+tops, workdir+parents, add-dirs+parents (glob-under only), plus
  variable/glob/substitution/inline-`sh -c` expansions that could resolve there
  (`"$VAR"/*`, `~`, `$HOME`, `$(...)`-only targets…). Rewrite via `"${VAR:?}"` guards or
  literal paths (guard doesn't clear dir-printing-substitution or normally-set vars like
  `$HOME`). Nested `(…)`/`{…}`/`$(…)`/backticks/`<…` and single-vs-double-quoted `-c`
  scripts inspected. Env opt-outs: `CLAUDE_CODE_DISABLE_SUBSTITUTION_RM_PROMPT`,
  `CLAUDE_CODE_DISABLE_INLINE_SHELL_RM_PROMPT`. Auto/bypass show a 2-min-countdown prompt
  (any key = wait; timeout = deny+replan; 3 timeouts = immediate-denies; new message
  resets); where no prompt possible (`-p`, SDK, VS Code/Desktop, pre-v2.1.281) → immediate
  deny; `CLAUDE_CODE_DISABLE_DANGEROUS_RM_TIMEOUT=1` restores classifier routing.
  PowerShell `Remove-Item` (`*`/`/*`/`\\*` incl. `$var/*` = always-deny) and `cmd rd/rmdir/
  del/erase` on system paths have their own always-deny tables
  (`CLAUDE_CODE_DISABLE_POWERSHELL_CMD_RM_DENY=1` opts out of the `cmd` half only).

## 6. Working dirs, settings scopes, trust

(source: permissions "Working directories", settings page, permissions "Project allow rules")

- Session starts rooted at launch dir (primary workdir); extend via `--add-dir` (startup),
  `/add-dir` (in-session; on a subdir also loads its skills/commands/agents w/o prompt,
  v2.1.257+), or `additionalDirectories` (persistent). Added dirs get same file rules as
  the root. `--add-dir`/SDK dirs also load skills/commands/subagents (project source;
  `--setting-sources`/`bare` can exclude); settings-file `additionalDirectories` grants
  file access ONLY. `/cd` MOVES the session (keeps history, loads new CLAUDE.md/settings/
  MCP/plugins/skills/agents/env/add-dirs, disconnects old's; gate with `Cd` rules).
- Settings scopes (high→low): **managed** (org: file/MDM/server-console/SDK-embedder) >
  **CLI `--settings`** (one session) > **project-local** `.claude/settings.local.json`
  (personal, git-excluded — Claude auto-excludes on first write) > **shared project**
  `.claude/settings.json` (commit to share) > **user** `~/.claude/settings.json`.
  Same list-keys MERGE across files (model-list keys excepted); deny-from-any-scope beats
  allow-from-any-other. Env vars are per-pair (e.g. `ANTHROPIC_MODEL` > `model` file value).
  `auto`/`bypassPermissions` `defaultMode` only from user/managed/flag (§3). Keys the shared
  file can never set (Scope=`User/local/managed`, `Global config`) + trust-gated keys
  (`allow`, `additionalDirectories`, most `env`, marketplaces) documented in settings
  troubleshooting. Strict JSON (no comments/trailing commas); bad file = Settings
  Error/Warning dialog (`-p` skips + continues; `claude doctor` details). Most edits
  hot-reload + fire `ConfigChange` hooks; `model/effort` need `/model|/effort`.
- Local-file placement: in git repos read/written at **repo root** (main-checkout root in
  worktrees; pre-v2.1.211: starting dir), except non-repo / home-root / Windows /
  foreign-owned-root cases; `/`-paths inside still anchor at session workdir. SDK
  `resolveSettings()` always uses starting dir.
- **Workspace trust**: project `.claude/settings.json` `allow` + `additionalDirectories`
  apply only after accepting the folder trust dialog (keyed at repo root / start dir /
  session-only under `$HOME`); deny/ask apply immediately. Own untracked `settings.local.json`
  skips trust (tracked/symlinked `.claude` = repo-supplied → trust applies). `-p`/SDK never
  prompt (parent-trust doesn't transfer; harden with `--setting-sources user`, `--bare`,
  `--settings '{"disableAllHooks":true}'`, `disabledMcpjsonServers`).
- Org lockdown: managed-only `allowManagedPermissionRulesOnly`, `disableBypassPermissionsMode`,
  `disableAutoMode`, managed `deny` (un-overridable, CLI flags included), ` isolating
  scopes; `/status` shows active sources, `claude doctor` shows rejections.

## 7. Sandbox (complementary to permissions)

(source: sandboxing page; permissions "How permissions interact with sandboxing")

- OS-enforced boundary (Seatbelt macOS / bubblewrap+socat Linux/WSL2; no native-Windows
  sandbox — use WSL2) around **Bash/PowerShell/Monitor + children only**. Read/Edit/WebFetch/
  MCP/hooks/LSP/status-line/`apiKeyHelper` run outside it; `!`-typed commands, `excludedCommands`,
  and `dangerouslyDisableSandbox` retries run outside it. Off by default (`/sandbox` panel or
  `sandbox.enabled`). Wraps via `@anthropic-ai/sandbox-runtime`.
- Default: writes confined to workdir(s)+`$TMPDIR`-redirected per-user tmp (+`allowWrite`);
  reads computer-wide minus denylists (so explicitly protect `~/.aws ~/.ssh` tokens via
  `sandbox.credentials` deny/mask + `denyRead`). `blockReadsOutsideWorkingDirectories` also
  strips home/user-file reads. Worktree sessions may write main `.git` (minus hooks/config).
  In-scope-but-protected config/code paths (`.claude/*` settings/skills/agents/hooks/workflows,
  `.mcp.json`, shell RCs, `.git/hooks|config`, fake top-level bare-repo files, `~/.claude*`)
  stay write-denied (0-byte Linux placeholders possible — `claude doctor` lists orphans).
  `filesystem.disabled:true` (user/managed/flag only; locked when managed `filesystem`/`deny`
  exists; ignored under `SUBPROCESS_ENV_SCRUB`) keeps network isolation, drops FS isolation.
- Network: no direct egress; proxy-env (`HTTP(S)_PROXY`) tools filtered by
  `allowedDomains` (+`WebFetch(domain:)` allows; `*.` + bare-`*` honored, other `*`
  positions fetch-only) / `deniedDomains` / `strictAllowlist` / `allowManagedDomainsOnly`;
  first-need prompts per permission mode (Yes = session, "don't ask again" = `WebFetch(domain:)`
  allow rule); loopback/link-local/metadata refused (except `localhost/*.localhost`; allow
  the literal IP to reach a loopback dev name); UDP/QUIC/ICMP + proxy-ignorers (ssh, most DB
  drivers) can't egress — exclude them. `tlsTerminate` enables credential-mask substitution
  (sentinel-in/s real-out on `injectHosts`); `mask` from trusted scopes only.
- **Auto-allow mode** (per-command sandbox mode, ≠ auto permission mode): sandboxed commands
  skip prompts; excluded/unsandboxed go through normal flow. Deny, critical-path, content-ask,
  and (in plan) bare-`Bash`-ask still apply. **Regular mode**: everything prompts normally.
  `autoAllowBashIfSandboxed` (default true) is the switch. Unsandboxed-retry needs
  `dangerouslyDisableSandbox` (Manual/acceptEdits prompt, auto→classifier, dontAsk deny;
  allow rules also approve it — add `Bash(dangerouslyDisableSandbox:true)` ask to catch it;
  `allowUnsandboxedCommands:false` = strict mode disables it + optionally `failIfUnavailable`
  refuses to start without a sandbox; typed-`!` commands sandboxed only in background
  sessions / `SUBPROCESS_ENV_SCRUB` Linux).
- Merging: FS arrays merge across scopes (locks excepted); `--setting-sources`-excluded
  sources contribute no FS/Edit/Read-deny; `excludedCommands` need `prefix *`, ALL chained
  commands must match, text-matched (scripts/`make`/abs paths slip past), redirects/`cd`/
  `$()` keep sandbox; admin-required sandbox (strict-false or domain-lock from managed/flag)
  ignores repo-loosening keys (repo deny + `autoAllowBash…` still apply). Org enforcement =
  managed `enabled + failIfUnavailable + allowUnsandboxedCommands:false` + credential denies
  (+ per-tool excludes); `allowManagedReadPathsOnly`/`allowManagedDomainsOnly` lock reads/hosts
  to managed entries; weaker-nesting/network/unix-socket/local-bind/AppleEvents knobs default
  off and repo-unescalatable for the dangerous ones.

## 8. Hooks (custom permission logic)

(source: permissions "Extend permissions with hooks")

- `PreToolUse` hooks run before the prompt (all tools but `EndConversation`) and can
  allow/prompt/deny; exit-2 blocks pre-rule-evaluation (beats allow). But deny/ask RULES
  are still evaluated regardless — deny always wins, ask still prompts even after hook-allow
  (incl. managed + plugin `hooks.json`). `requiresUserInteraction` MCP + org-pinned-ask
  connectors still prompt after hook-allow. `tool.check`-handling mods answer AFTER
  rules+hooks and can approve ask/hook-blocks/auto-classifier (deny-hold depends on managed/
  plan: default holds on managed/Team/Ent; org can flip). Use hooks for command-text-aware
  blocks (e.g. protected-file edits) that prefix rules can't express.

## 9. Reading this against `rem` (repo-local map)

- `rem` today: 5 hard-coded session-local modes (`plan/manual/auto/edit/yolo`, ADR-0011),
  pure `classify(tool,args,root) -> Allow|Confirm|Deny` (ADR-0002), choke-point hook
  (ADR-0001), interrupt/resume approvals (ADR-0003). TODO tracks Claude/Codex/Pi parity.
- Closest Claude analogues: `rem manual`≈Claude Manual; `rem auto`≈Claude acceptEdits
  (in-scope file+dev-command fast-paths) rather than Claude auto (no classifier in `rem`);
  `rem yolo`≈`bypassPermissions` minus Claude's ask/critical-path/protected-path floors
  (rem keeps annihilator+exfiltration deny — same spirit); `rem plan`≈Claude plan (read-only
  + deny-with-replan).
- Biggest gaps visible from these sources if parity is the goal: rule system
  (`allow/ask/deny` + gitignore file patterns + `domain:` fetch rules + deny≻ask≻allow
  precedence + settings-scope merge + trust-gated project allows), compound-command +
  wrapper-aware Bash matching, redirect/tee target checks, symlink requested+resolved
  checks, WebFetch-domain↔sandbox domain merge, `Cd`-gated `/cd`, protected/critical-path
  tables with per-mode outcomes, classifier-driven auto (env/`hard|soft_deny`/`allow`,
  per-command domains, Recently-denied retry, repeated-block fallback), sandbox
  (auto-allow vs regular, excludedCommands, strict/fail-closed, credential mask/deny),
  PreToolUse/PermissionDenied hooks + mods precedence, first-outside-read prompt,
  per-conversation plan-approve handoff (`Ctrl+G`, clear-context option).
