use std::time::Duration;

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

use super::ToolError;

const TIMEOUT_SECS: u64 = 30;
const MAX_BYTES: usize = 8000;

#[derive(Debug, Deserialize)]
pub struct GitStatusArgs {}

#[derive(Debug)]
pub struct GitStatusTool;

impl Tool for GitStatusTool {
    const NAME: &'static str = "git_status";
    type Args = GitStatusArgs;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Run `git status --short` in the project directory. 30s timeout. Output truncated to 8000 bytes.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn call(&self, _ctx: &mut ToolContext, _args: Self::Args) -> Result<String, ToolError> {
        run_git(vec!["status".to_string(), "--short".to_string()]).await
    }
}

#[derive(Debug, Deserialize)]
pub struct GitDiffArgs {
    pub path: Option<String>,
    pub staged: Option<bool>,
}

#[derive(Debug)]
pub struct GitDiffTool;

impl Tool for GitDiffTool {
    const NAME: &'static str = "git_diff";
    type Args = GitDiffArgs;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Run `git diff` in the project directory. Optional `path` limits to one file, optional `staged` shows `git diff --staged`. 30s timeout, truncated to 8000 bytes.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Optional file path to limit the diff to" },
                "staged": { "type": "boolean", "description": "If true, run `git diff --staged`. Omit for unstaged." }
            }
        })
    }

    async fn call(&self, _ctx: &mut ToolContext, args: Self::Args) -> Result<String, ToolError> {
        let mut argv: Vec<String> = vec!["diff".to_string()];
        if args.staged.unwrap_or(false) {
            argv.push("--staged".to_string());
        }
        if let Some(p) = args.path.as_deref()
            && !p.is_empty()
        {
            argv.push("--".to_string());
            argv.push(p.to_string());
        }
        run_git(argv).await
    }
}

/// Run `git <args>` in `root` with a 30s timeout, truncating output to 8000 bytes.
pub(crate) async fn run_git_in(
    root: &std::path::Path,
    argv: Vec<String>,
) -> Result<String, ToolError> {
    let child = tokio::process::Command::new("git")
        .current_dir(root)
        .args(&argv)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ToolError(format!("git spawn: {e}")))?;

    let output = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), child.wait_with_output())
        .await
        .map_err(|_| {
            ToolError(format!(
                "git {}: timed out after {TIMEOUT_SECS}s",
                argv.join(" ")
            ))
        })?
        .map_err(|e| ToolError(format!("git wait: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError(format!(
            "git {} failed (exit {}): {}",
            argv.join(" "),
            output
                .status
                .code()
                .map_or("signal".to_string(), |c| c.to_string()),
            stderr.trim()
        )));
    }

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str("[stderr]\n");
        combined.push_str(&stderr);
    }

    let mut note = String::new();
    if combined.len() > MAX_BYTES {
        combined.truncate(MAX_BYTES);
        note = format!(" [truncated to {MAX_BYTES} bytes]");
    }

    if combined.trim().is_empty() {
        Ok("(clean)".to_string())
    } else {
        Ok(format!("{combined}{note}"))
    }
}

/// Convenience wrapper: runs `git <args>` at the process working directory.
/// Preserves the historical tool behaviour (no `current_dir` set).
pub async fn run_git(argv: Vec<String>) -> Result<String, ToolError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    run_git_in(&cwd, argv).await
}

/// Restore the working tree in `root` to the state a checkpointed `git_diff`
/// describes (the `/undo` + `/rewind` file path): first
/// `git checkout -- .` discards every tracked change (the turn's edits),
/// then the snapshot diff is re-applied so pre-turn uncommitted work survives.
/// `(clean)` / empty snapshots skip the re-apply (pre-turn tree == HEAD).
/// Sync on purpose: `/undo` and `/rewind` run on the TUI event loop, which
/// already performs blocking session I/O in its command handlers. The diff
/// is capped at 8000 bytes, so the stdin write cannot deadlock.
pub fn restore_diff(root: &std::path::Path, git_diff: &str) -> Result<String, String> {
    git_status_in(root, &["checkout", "--", "."]).map_err(|e| format!("restore checkout: {e}"))?;

    let trimmed = git_diff.trim();
    if trimmed.is_empty() || trimmed == "(clean)" {
        return Ok("files reset to HEAD (snapshot was clean)".to_string());
    }

    // Feed the patch on stdin: no temp file, no quoting hazards.
    let mut child = std::process::Command::new("git")
        .current_dir(root)
        .args(["apply", "--whitespace=nowarn", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("restore apply spawn: {e}"))?;

    use std::io::Write;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "restore apply: no stdin".to_string())?;
    // Patches must end with a newline; trimmed captures lose it.
    let mut patch = git_diff.to_string();
    if !patch.ends_with('\n') {
        patch.push('\n');
    }
    stdin
        .write_all(patch.as_bytes())
        .map_err(|e| format!("restore apply write: {e}"))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|e| format!("restore apply wait: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "restore apply failed (exit {}): {}",
            output
                .status
                .code()
                .map_or("signal".to_string(), |c| c.to_string()),
            stderr.trim()
        ));
    }
    Ok("files restored to snapshot state".to_string())
}

/// Sync `git <args>` in `root`: stdout trimmed, stderr attached on failure.
/// Used by [`restore_diff`]; the async [`run_git_in`] serves `chat()`.
fn git_status_in(root: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| format!("git spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Run `git <args>` in `dir`, return stdout (trimmed). Panics on failure
    /// unless `allow_fail`.
    fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Temp git repo with one committed file. Returns (dir, file path).

    #[tokio::test]
    async fn restore_diff_reapplies_pre_turn_state_after_mutation() {
        // Repo with base committed, a pre-turn uncommitted edit (the diff we
        // snapshot), then a "turn" mutation that must be rolled back.
        let dir = std::env::temp_dir().join(format!("rem-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "base\n").unwrap();
        git_in(&dir, &["init", "-q"]);
        git_in(&dir, &["add", "."]);
        git_in(
            &dir,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );

        // Pre-turn: an uncommitted edit exists. Snapshot its diff.
        std::fs::write(&file, "pre-turn edit\n").unwrap();
        let pre_diff = run_git_in(&dir, vec!["diff".into()]).await.unwrap();
        assert!(pre_diff.contains("pre-turn edit"), "got: {pre_diff}");

        // The turn mutates further.
        std::fs::write(&file, "turn edit\n").unwrap();

        // Restore: working tree returns to the pre-turn state.
        restore_diff(&dir, &pre_diff).expect("restore works");
        let after = std::fs::read_to_string(&file).unwrap();
        assert_eq!(after, "pre-turn edit\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn restore_diff_clean_snapshot_discards_turn_changes() {
        // Pre-turn was clean: the REAL capture path (run_git_in) yields the
        // "(clean)" sentinel, and restore discards the turn's edits to HEAD.
        let dir = std::env::temp_dir().join(format!("rem-restore-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "base\n").unwrap();
        git_in(&dir, &["init", "-q"]);
        git_in(&dir, &["add", "."]);
        git_in(
            &dir,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );

        let pre_diff = run_git_in(&dir, vec!["diff".into()]).await.unwrap();
        assert_eq!(pre_diff, "(clean)");

        std::fs::write(&file, "turn edit\n").unwrap();

        restore_diff(&dir, &pre_diff).expect("restore works");
        let after = std::fs::read_to_string(&file).unwrap();
        assert_eq!(after, "base\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
