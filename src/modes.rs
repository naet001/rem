//! Claude permission modes — 6 modes with correct cycle order, labels, and start-mode resolution.
//!
//! Config value (what appears in `name()`) matches Claude Code exactly.
//! Cycle: auto → default → acceptEdits → plan → (optional: bypassPermissions, auto).
//! start-mode: flag > defaultMode(scope-restricted) > built-in default.
//! Kill switches: `permissions.disableAutoMode: "disable"` / `permissions.disableBypassPermissionsMode: "disable"`.
//!
//! Map from old rem names (kept as aliases for migration):
//!   old "edit"  → new "acceptEdits"  (config "acceptEdits")
//!   old "yolo"  → new "bypassPermissions" (config "bypassPermissions")
//!   old "manual"→ new "default" (config "default")
//!   old "plan"  → new "plan" (config "plan")
//!   old "auto"  → new "auto" (config "auto")
//!   "dontAsk"  is new; has no old alias in the 5-mode set.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Manual mode (config value: `"default"`). Stop-and-ask before most
    /// edits, shell commands, and network.  Status bar: `⏸ manual mode on`.
    #[default]
    Manual,
    /// `acceptEdits` mode (config value: `"acceptEdits"`). Auto-approve
    /// file create/edit inside the working directory(s) plus safe-env /
    /// wrapper prefixes.  Everything else follows the normal prompt flow.
    ///  Status bar: `⏵⏵ accept edits on`.
    AcceptEdits,
    /// `plan` mode (config value: `"plan"`).  Research + propose, no edits
    /// until the plan is approved.  Shell during planning depends on the
    /// session (bypass-available interactive terminal → neither classifier
    /// nor prompt; auto+useAutoModeDuringPlan → classifier reviews; else
    /// prompt for non-read-only).  Status bar: `⏸ plan mode on`.
    Plan,
    /// `auto` mode (config value: `"auto"`).  A separate classifier model
    /// reviews actions instead of the human.  Explicit `ask` rules still
    /// force a prompt.  Since v2.1.283 the built-in default for interactive
    /// terminal / VS Code on all plans/providers.  Status bar:
    /// `⏵⏵ auto mode on`.
    Auto,
    /// `dontAsk` mode (config value: `"dontAsk"`).  Auto-deny anything that
    /// would prompt (reads + allow / hook-approved still run).  AskUserQuestion,
    /// org-pinned-ask MCP tools, `requiresUserInteraction` MCP tools are
    /// denied.  Status bar: `⏵⏵ don't ask on`.
    DontAsk,
    /// `bypassPermissions` mode (config value: `"bypassPermissions"`).
    /// No prompts / checks; tool calls execute immediately **including
    /// protected-path writes**.  Still prompts/blocks: explicit ask rules,
    /// critical-path `rm`/`rmdir`, `requiresUserInteraction`, org-pinned
    /// ask connectors, cross-session-messaging safeguards,
    /// `blockReadsOutsideWorkingDirectories` reads, unsandboxed-retry
    /// approval, public-artifact reads.  Opt-in at launch only.
    ///  Status bar: `⏵⏵ bypass permissions on`.
    BypassPermissions,
}

impl Mode {
    /// All six modes. Order matters for `next()` cycle.
    pub const ALL: [Mode; 6] = [
        Mode::Auto,
        Mode::Manual,
        Mode::AcceptEdits,
        Mode::Plan,
        Mode::DontAsk,
        Mode::BypassPermissions,
    ];

    /// Claude-mode cycle: `auto → default → acceptEdits → plan → don'tAsk → bypassPermissions → auto`.
    /// Optionals `bypassPermissions` and `dontAsk` slot in after `plan`;
    /// `bypassPermissions` is first on the way to `auto`, `dontAsk` is last.
    /// `next()` wraps around; calling it from `BypassPermissions` returns `Auto`.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        let next_i = (i + 1) % Self::ALL.len();
        Self::ALL[next_i]
    }

    /// Config value — what appears in the status bar / `/permissions`
    /// dropdown / `--mode` flag parsing.
    /// Staged API: callers land in plan Tasks 24 (`/permissions` dialog)
    /// and 28/29 (headless flags + resume).
    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Mode::Manual => "default",
            Mode::AcceptEdits => "acceptEdits",
            Mode::Plan => "plan",
            Mode::Auto => "auto",
            Mode::DontAsk => "dontAsk",
            Mode::BypassPermissions => "bypassPermissions",
        }
    }

    /// Parse `--mode <name>` (case-insensitive). Accepts both Claude
    /// config values and legacy rem names for migration.
    pub fn from_name(raw: &str) -> Option<Self> {
        let s = raw.trim().to_lowercase();
        // Claude config values
        match s.as_str() {
            "default" => Some(Self::Manual),
            "manual" => Some(Self::Manual),
            "acceptedits" => Some(Self::AcceptEdits),
            "edit" => Some(Self::AcceptEdits), // legacy rem alias
            "plan" => Some(Self::Plan),
            "auto" => Some(Self::Auto),
            "dontask" => Some(Self::DontAsk),
            "bypasspermissions" => Some(Self::BypassPermissions),
            "bypass" => Some(Self::BypassPermissions), // short alias
            "yolo" => Some(Self::BypassPermissions),   // legacy rem alias
            _ => None,
        }
    }

    /// Status-bar label — the human-readable status text shown in the
    /// status row / mode-change notice (Claude's exact status phrasing).
    pub fn label(self) -> &'static str {
        match self {
            Mode::Manual => "⏸ manual mode on",
            Mode::AcceptEdits => "⏵⏵ accept edits on",
            Mode::Plan => "⏸ plan mode on",
            Mode::Auto => "⏵⏵ auto mode on",
            Mode::DontAsk => "⏵⏵ don't ask on",
            Mode::BypassPermissions => "⏵⏵ bypass permissions on",
        }
    }

    /// Footer display: `label()` without the leading status emoji, since
    /// the footer renders its own `▶` marker. Keeps the existing compact
    /// `▶ manual · model · effort` shape while using the Claude status
    /// wording (per plan Task 23: footer uses `label()`).
    pub fn footer_text(self) -> String {
        format!("▶ {}", self.label().trim_start_matches(['⏸', '⏵', ' ']))
    }

    /// Start-mode resolution: flag > `permissions.defaultMode`
    /// (scope-restricted) > built-in default (`Auto`).
    ///
    /// `scope` determines whether `defaultMode` is honoured:
    /// - `Scope::User`, `Scope::Managed`, `Scope::Cli`: all values apply.
    /// - `Scope::Project`, `Scope::Local`: `auto`/`bypassPermissions` are
    ///   ignored (`auto` → built-in default, `bypassPermissions` → Manual);
    ///   every other value still applies.
    /// - Unknown `defaultMode` string → `Manual` (fail safe, never silently
    ///   escalate to the more permissive built-in default).
    /// - Absent `defaultMode` → built-in default (`Auto`).
    ///
    /// Staged API: wired to settings/headless in plan Tasks 9, 28, 29.
    #[allow(dead_code)]
    pub fn resolve_start_mode(
        flag: Option<Mode>,
        default_mode: Option<&str>,
        scope: Scope,
    ) -> Mode {
        // 1. CLI / flag wins outright
        if let Some(m) = flag {
            return m;
        }
        // 2. defaultMode from settings
        let Some(dm) = default_mode else {
            return Self::built_in_default();
        };
        let Some(parsed) = Self::from_name(dm) else {
            return Mode::Manual;
        };
        // 3. Project/Local scope cannot activate auto or bypass.
        if matches!(scope, Scope::Project | Scope::Local) {
            match parsed {
                Mode::Auto => return Self::built_in_default(),
                Mode::BypassPermissions => return Mode::Manual,
                _ => return parsed,
            }
        }
        parsed
    }

    /// Built-in default when no flag and no `defaultMode` apply.
    /// Since v2.1.283 the built-in default for interactive terminal.
    /// Staged API: see `resolve_start_mode`.
    #[allow(dead_code)]
    pub fn built_in_default() -> Mode {
        Mode::Auto
    }

    /// Kill-switch helpers (plan Task 4). `PermissionsSettings` lands in
    /// Phase C; these take the raw setting strings now and get rewired
    /// in Phase C task 9.
    /// Staged API: see `resolve_start_mode`.
    #[allow(dead_code)]
    pub fn auto_disabled(disable_auto_mode: Option<&str>) -> bool {
        matches!(disable_auto_mode, Some(v) if v == "disable")
    }

    /// Kill-switch helpers (plan Task 4). `PermissionsSettings` lands in
    /// Phase C; these take the raw setting strings now and get rewired
    /// in Phase C task 9.
    /// Staged API: see `resolve_start_mode`.
    #[allow(dead_code)]
    pub fn bypass_disabled(disable_bypass_permissions_mode: Option<&str>) -> bool {
        matches!(disable_bypass_permissions_mode, Some(v) if v == "disable")
    }
}

/// Scope of settings that determines whether `defaultMode` is honoured.
/// Staged API: see `resolve_start_mode`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Managed,
    Cli,
    Local,
    Project,
    User,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_order_matches_claude() {
        // auto → default → acceptEdits → plan → don'tAsk → bypassPermissions → auto
        assert_eq!(Mode::Auto.next(), Mode::Manual);
        assert_eq!(Mode::Manual.next(), Mode::AcceptEdits);
        assert_eq!(Mode::AcceptEdits.next(), Mode::Plan);
        assert_eq!(Mode::Plan.next(), Mode::DontAsk);
        assert_eq!(Mode::DontAsk.next(), Mode::BypassPermissions);
        assert_eq!(Mode::BypassPermissions.next(), Mode::Auto);
    }

    #[test]
    fn name_returns_claude_config_values() {
        assert_eq!(Mode::Manual.name(), "default");
        assert_eq!(Mode::AcceptEdits.name(), "acceptEdits");
        assert_eq!(Mode::Plan.name(), "plan");
        assert_eq!(Mode::Auto.name(), "auto");
        assert_eq!(Mode::DontAsk.name(), "dontAsk");
        assert_eq!(Mode::BypassPermissions.name(), "bypassPermissions");
    }

    #[test]
    fn from_name_accepts_claude_and_legacy() {
        // Claude values
        assert_eq!(Mode::from_name("default"), Some(Mode::Manual));
        assert_eq!(Mode::from_name("acceptEdits"), Some(Mode::AcceptEdits));
        assert_eq!(Mode::from_name("plan"), Some(Mode::Plan));
        assert_eq!(Mode::from_name("auto"), Some(Mode::Auto));
        assert_eq!(Mode::from_name("dontAsk"), Some(Mode::DontAsk));
        assert_eq!(
            Mode::from_name("bypassPermissions"),
            Some(Mode::BypassPermissions)
        );

        // Legacy rem aliases (kept for migration)
        assert_eq!(Mode::from_name("manual"), Some(Mode::Manual));
        assert_eq!(Mode::from_name("edit"), Some(Mode::AcceptEdits));
        assert_eq!(Mode::from_name("yolo"), Some(Mode::BypassPermissions));

        // Unknown
        assert_eq!(Mode::from_name("turbo"), None);
    }

    #[test]
    fn label_returns_claude_status_text() {
        assert_eq!(Mode::Manual.label(), "⏸ manual mode on");
        assert_eq!(Mode::AcceptEdits.label(), "⏵⏵ accept edits on");
        assert_eq!(Mode::Plan.label(), "⏸ plan mode on");
        assert_eq!(Mode::Auto.label(), "⏵⏵ auto mode on");
        assert_eq!(Mode::DontAsk.label(), "⏵⏵ don't ask on");
        assert_eq!(Mode::BypassPermissions.label(), "⏵⏵ bypass permissions on");
    }

    #[test]
    fn footer_text_uses_label_with_footer_marker() {
        // Footer renders its own ▶ marker + label wording without the
        // status emoji (plan Task 23: footer uses label()).
        assert_eq!(Mode::Manual.footer_text(), "▶ manual mode on");
        assert_eq!(Mode::Auto.footer_text(), "▶ auto mode on");
        assert!(Mode::Manual.footer_text().contains("▶ manual"));
    }

    #[test]
    fn resolve_start_mode_scope_restrictions() {
        // Plan Task 4: auto/bypassPermissions from Project/Local scope are
        // ignored (auto→built-in Auto, bypass→Manual).
        assert_eq!(
            Mode::resolve_start_mode(None, Some("auto"), Scope::Project),
            Mode::Auto // built-in
        );
        assert_eq!(
            Mode::resolve_start_mode(None, Some("bypassPermissions"), Scope::Project),
            Mode::Manual // bypass from Project falls back
        );
        assert_eq!(
            Mode::resolve_start_mode(None, Some("auto"), Scope::User),
            Mode::Auto // User honours it
        );
        assert_eq!(
            Mode::resolve_start_mode(None, Some("bypassPermissions"), Scope::User),
            Mode::BypassPermissions
        );
        // Cli always honours
        assert_eq!(
            Mode::resolve_start_mode(None, Some("auto"), Scope::Cli),
            Mode::Auto
        );
        // Managed always honours
        assert_eq!(
            Mode::resolve_start_mode(None, Some("bypassPermissions"), Scope::Managed),
            Mode::BypassPermissions
        );
        // Local scope behaves like Project
        assert_eq!(
            Mode::resolve_start_mode(None, Some("bypassPermissions"), Scope::Local),
            Mode::Manual
        );
        assert_eq!(
            Mode::resolve_start_mode(None, Some("auto"), Scope::Local),
            Mode::Auto // built-in
        );
        // Non-restricted values still apply from Project/Local
        assert_eq!(
            Mode::resolve_start_mode(None, Some("plan"), Scope::Project),
            Mode::Plan
        );
        assert_eq!(
            Mode::resolve_start_mode(None, Some("acceptEdits"), Scope::Local),
            Mode::AcceptEdits
        );
        // Unknown string fails safe to Manual; absent falls to built-in Auto
        assert_eq!(
            Mode::resolve_start_mode(None, Some("turbo"), Scope::User),
            Mode::Manual
        );
        assert_eq!(
            Mode::resolve_start_mode(None, None, Scope::User),
            Mode::built_in_default()
        );
        assert_eq!(Mode::built_in_default(), Mode::Auto);
        // Flag wins outright
        assert_eq!(
            Mode::resolve_start_mode(Some(Mode::Plan), Some("auto"), Scope::User),
            Mode::Plan
        );
        // Kill switches
        assert!(Mode::auto_disabled(Some("disable")));
        assert!(!Mode::auto_disabled(None));
        assert!(Mode::bypass_disabled(Some("disable")));
        assert!(!Mode::bypass_disabled(Some("enable")));
    }
}
