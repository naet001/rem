pub mod bash;
pub mod edit;
pub mod git;
pub mod glob_tool;
pub mod grep;
pub mod list_directory;
pub mod read;
pub mod task;
pub mod todo;
pub mod web;
pub mod write;

pub use bash::BashTool;
pub use edit::EditTool;
pub use git::{GitDiffTool, GitStatusTool};
pub use glob_tool::GlobTool;
pub use grep::GrepTool;
pub use list_directory::ListDirectoryTool;
pub use read::ReadTool;
pub use task::{
    BackgroundInfo, BackgroundStatus, BackgroundStore, TaskChildRunner, TaskTool, task_preview,
};
pub use todo::{TodoItem, TodoStatus, TodoWriteTool};
pub use web::{WebFetchTool, WebSearchTool};
pub use write::WriteTool;

use std::fmt;

/// Shared error type for all tools. Rig's `Tool::Error` must implement
/// `std::error::Error`, so we wrap a plain message.
#[derive(Debug)]
pub struct ToolError(pub String);

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ToolError {}

/// Lenient `Option<usize>` deserializer: accepts JSON integers *and*
/// floats like `120.0` (which models often emit for `"type": "number"`
/// schemas). Fractional values are truncated (`120.9` -> `120`).
/// Negative / non-finite values are rejected.
///
/// This fixes: `invalid type: floating point 120.0, expected usize`.
pub(crate) fn opt_usize_lenient<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let v: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match v {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => {
            if let Some(i) = n.as_u64() {
                Ok(Some(i as usize))
            } else if let Some(f) = n.as_f64() {
                if !f.is_finite() || f < 0.0 {
                    return Err(serde::de::Error::custom(format!(
                        "expected non-negative integer, got {f}"
                    )));
                }
                Ok(Some(f as usize))
            } else {
                Err(serde::de::Error::custom(format!(
                    "expected non-negative integer, got {n}"
                )))
            }
        }
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected integer, got {other}"
        ))),
    }
}

/// Lenient `Option<u8>` deserializer: same float-tolerance as
/// [`opt_usize_lenient`], saturating at `u8::MAX` so oversized values
/// still reach downstream `.clamp(...)` logic instead of failing to parse.
pub(crate) fn opt_u8_lenient<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let v: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match v {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => {
            if let Some(i) = n.as_u64() {
                Ok(Some(i.min(u8::MAX as u64) as u8))
            } else if let Some(f) = n.as_f64() {
                if !f.is_finite() || f < 0.0 {
                    return Err(serde::de::Error::custom(format!(
                        "expected non-negative integer, got {f}"
                    )));
                }
                Ok(Some((f as u64).min(u8::MAX as u64) as u8))
            } else {
                Err(serde::de::Error::custom(format!(
                    "expected non-negative integer, got {n}"
                )))
            }
        }
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected integer, got {other}"
        ))),
    }
}

/// Context lines around each change hunk (same `-U3` default as `git diff`).
const DIFF_CONTEXT: usize = 3;
/// Total-line ceiling for the exact Myers pass: past this, fall back to one
/// prefix/suffix-trimmed hunk so time and memory stay bounded on big files.
const DIFF_MYERS_MAX_LINES: usize = 1000;

/// Minimal line-based unified diff of `old` → `new` for one file, with
/// `--- a/<path>` / `+++ b/<path>` headers so the transcript's tinted
/// `patch_row` renders it exactly like `git_diff` output. Empty when the
/// texts are identical (the caller then prints just the summary line).
pub(crate) fn unified_diff(path: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    if a.len() + b.len() > DIFF_MYERS_MAX_LINES {
        push_coarse_hunk(&mut out, &a, &b);
    } else {
        push_hunks(&mut out, &a, &b, &myers_ops(&a, &b));
    }
    out
}

/// Group a Myers script into `-U3` hunks: every changed op seeds a window of
/// [`DIFF_CONTEXT`] keeps on each side; overlapping windows merge.
fn push_hunks(out: &mut String, a: &[&str], b: &[&str], ops: &[DiffOp]) {
    let mut ai = 0usize;
    let mut bi = 0usize;
    let mut at: Vec<(DiffOp, usize, usize)> = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            DiffOp::Keep => {
                at.push((*op, ai, bi));
                ai += 1;
                bi += 1;
            }
            DiffOp::Del => {
                at.push((*op, ai, bi));
                ai += 1;
            }
            DiffOp::Ins => {
                at.push((*op, ai, bi));
                bi += 1;
            }
        }
    }
    let changes: Vec<usize> = at
        .iter()
        .enumerate()
        .filter(|(_, (op, _, _))| *op != DiffOp::Keep)
        .map(|(i, _)| i)
        .collect();
    if changes.is_empty() {
        // `lines()` hides trailing-newline/line-ending-only changes: show a
        // context tail plus a note instead of an empty diff.
        let ctx = a.len().min(DIFF_CONTEXT);
        let start = a.len() - ctx;
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            start + 1,
            ctx,
            start + 1,
            ctx
        ));
        for line in &a[start..] {
            out.push(' ');
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("\\ (only line endings or trailing newline differ)\n");
        return;
    }
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut gs = changes[0].saturating_sub(DIFF_CONTEXT);
    let mut ge = (changes[0] + DIFF_CONTEXT + 1).min(at.len());
    for &c in &changes[1..] {
        let s = c.saturating_sub(DIFF_CONTEXT);
        let e = (c + DIFF_CONTEXT + 1).min(at.len());
        if s <= ge {
            ge = ge.max(e);
        } else {
            groups.push((gs, ge));
            gs = s;
            ge = e;
        }
    }
    groups.push((gs, ge));
    for (s, e) in groups {
        let slice = &at[s..e];
        let a_start = slice[0].1;
        let b_start = slice[0].2;
        let a_len = slice.iter().filter(|(op, _, _)| *op != DiffOp::Ins).count();
        let b_len = slice.iter().filter(|(op, _, _)| *op != DiffOp::Del).count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            a_start + 1,
            a_len,
            b_start + 1,
            b_len
        ));
        for (op, i, j) in slice {
            match op {
                DiffOp::Keep => {
                    out.push(' ');
                    out.push_str(a[*i]);
                }
                DiffOp::Del => {
                    out.push('-');
                    out.push_str(a[*i]);
                }
                DiffOp::Ins => {
                    out.push('+');
                    out.push_str(b[*j]);
                }
            }
            out.push('\n');
        }
    }
}

/// Single-hunk fallback for large inputs: trim the common prefix/suffix and
/// show the middle as one replace, with [`DIFF_CONTEXT`] context each side.
fn push_coarse_hunk(out: &mut String, a: &[&str], b: &[&str]) {
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < (a.len() - pre).min(b.len() - pre) && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let ctx = pre.min(DIFF_CONTEXT);
    let post = suf.min(DIFF_CONTEXT);
    let a_start = pre - ctx;
    let b_start = pre - ctx;
    let a_end = a.len() - suf + post;
    let b_end = b.len() - suf + post;
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        a_start + 1,
        a_end - a_start,
        b_start + 1,
        b_end - b_start
    ));
    for line in &a[a_start..pre] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    for line in &a[pre..a.len() - suf] {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in &b[pre..b.len() - suf] {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    for line in &b[b.len() - suf..b.len() - suf + post] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiffOp {
    Keep,
    Del,
    Ins,
}

/// Myers O(ND) line script turning `a` into `b`. Empty sides short-circuit;
/// the caller keeps total lines under [`DIFF_MYERS_MAX_LINES`] so the
/// backtrack trace stays small.
fn myers_ops(a: &[&str], b: &[&str]) -> Vec<DiffOp> {
    let n = a.len() as isize;
    let m = b.len() as isize;
    if n == 0 {
        return vec![DiffOp::Ins; m as usize];
    }
    if m == 0 {
        return vec![DiffOp::Del; n as usize];
    }
    let max = (n + m) as usize;
    let off = max as isize;
    // `v[k]` = furthest x on diagonal k after the previous round; each
    // round's copy is kept so the shortest path can be walked back.
    let mut v = vec![0isize; 2 * max + 1];
    let mut trace = Vec::new();
    let mut end_d = 0isize;
    let max_d = n + m;
    'outer: for d in 0..=max_d {
        let mut cur = v.clone();
        for k in (-d..=d).step_by(2) {
            let idx = (off + k) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            cur[idx] = x;
            if x >= n && y >= m {
                trace.push(cur);
                end_d = d;
                break 'outer;
            }
        }
        trace.push(cur.clone());
        v = cur;
    }
    // Walk back from (n, m): the run of keeps after each edit, then the
    // single edit itself — a deletion when x stepped, else an insertion.
    let mut ops = Vec::with_capacity((n + m) as usize);
    let (mut x, mut y) = (n, m);
    for d in (1..=end_d).rev() {
        let prev = &trace[(d - 1) as usize];
        let k = x - y;
        let pk = (off + k) as usize;
        let k_prev = if k == -d || (k != d && prev[pk - 1] < prev[pk + 1]) {
            k + 1
        } else {
            k - 1
        };
        let x_prev = prev[(off + k_prev) as usize];
        let y_prev = x_prev - k_prev;
        while x > x_prev && y > y_prev {
            ops.push(DiffOp::Keep);
            x -= 1;
            y -= 1;
        }
        if x > x_prev {
            ops.push(DiffOp::Del);
        } else {
            ops.push(DiffOp::Ins);
        }
        x = x_prev;
        y = y_prev;
    }
    while x > 0 && y > 0 {
        ops.push(DiffOp::Keep);
        x -= 1;
        y -= 1;
    }
    ops.reverse();
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_marks_change_with_headers_and_hunk() {
        let d = unified_diff("f.txt", "a\nb\nc\n", "a\nB\nc\n");
        let lines: Vec<&str> = d.lines().collect();
        assert!(lines.contains(&"--- a/f.txt"), "got:\n{d}");
        assert!(lines.contains(&"+++ b/f.txt"), "got:\n{d}");
        assert!(lines.contains(&"-b"), "got:\n{d}");
        assert!(lines.contains(&"+B"), "got:\n{d}");
        assert!(lines.iter().any(|l| l.starts_with("@@ ")), "got:\n{d}");
    }

    #[test]
    fn unified_diff_new_file_is_all_added() {
        let d = unified_diff("n.txt", "", "x\ny\n");
        assert!(d.lines().any(|l| l == "+x"), "got:\n{d}");
        assert!(
            !d.lines()
                .any(|l| l.starts_with('-') && !l.starts_with("---")),
            "got:\n{d}"
        );
    }

    #[test]
    fn unified_diff_identical_is_empty() {
        assert_eq!(unified_diff("f", "a\n", "a\n"), "");
    }

    #[test]
    fn unified_diff_splits_scattered_changes_into_hunks() {
        let old: String = (0..20).map(|i| format!("l{i}\n")).collect();
        let mut new_lines: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
        new_lines[2] = "CHANGED-A".to_string();
        new_lines[17] = "CHANGED-B".to_string();
        let new = new_lines.join("\n") + "\n";
        let d = unified_diff("f", &old, &new);
        // `@@` opens and closes each hunk header: two hunks, not one middle.
        assert_eq!(d.matches("@@").count(), 4, "got:\n{d}");
        assert!(d.lines().any(|l| l == "-l2"), "got:\n{d}");
        assert!(d.lines().any(|l| l == "+CHANGED-A"), "got:\n{d}");
        assert!(d.lines().any(|l| l == "-l17"), "got:\n{d}");
        assert!(d.lines().any(|l| l == "+CHANGED-B"), "got:\n{d}");
    }

    #[test]
    fn unified_diff_trailing_newline_only_still_shows() {
        let d = unified_diff("f", "a\n", "a");
        assert!(!d.is_empty(), "newline-only change must not diff empty");
    }

    #[test]
    fn unified_diff_large_inputs_use_single_hunk() {
        let old: String = (0..1200).map(|i| format!("l{i}\n")).collect();
        let new = old.replacen("l500\n", "CHANGED\n", 1);
        let d = unified_diff("big", &old, &new);
        assert!(
            d.lines().any(|l| l == "+CHANGED"),
            "got head:\n{}",
            &d[..d.len().min(200)]
        );
        assert_eq!(d.matches("@@").count(), 2, "one coarse hunk");
    }
}
