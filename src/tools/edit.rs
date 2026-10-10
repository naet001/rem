use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

use super::ToolError;

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct SingleEdit {
    pub oldText: String,
    pub newText: String,
}

#[derive(Debug, Deserialize)]
pub struct EditArgs {
    pub path: String,
    pub edits: Vec<SingleEdit>,
    /// When true, allow whitespace-relaxed (`apply_patch`-style) matching:
    /// `oldText` must still be recognized line-by-line, but intra-line
    /// whitespace differences (and leading/trailing whitespace) are
    /// tolerated. Exact matches still win when they exist.
    #[serde(default)]
    pub fuzzy: bool,
}

#[derive(Debug)]
pub struct EditTool;

impl Tool for EditTool {
    const NAME: &'static str = "edit";
    type Args = EditArgs;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Apply string replacements to a file. Each `oldText` must occur exactly once or the whole call fails; with `fuzzy:true` matching relaxes to whitespace-insensitive lines. Edits apply sequentially.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path of the file to edit" },
                "fuzzy": { "type": "boolean", "description": "Relax matching to whitespace-insensitive lines (apply_patch-style); exact matches still win" },
                "edits": {
                    "type": "array",
                    "description": "Replacements to apply in order",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": { "type": "string", "description": "Exact text to find (must be unique in the file)" },
                            "newText": { "type": "string", "description": "Replacement text" }
                        },
                        "required": ["oldText", "newText"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }

    async fn call(&self, _ctx: &mut ToolContext, args: Self::Args) -> Result<String, ToolError> {
        let old = std::fs::read_to_string(&args.path)
            .map_err(|e| ToolError(format!("edit {}: {e}", args.path)))?;
        let mut content = old.clone();
        let mut applied = 0;
        for e in &args.edits {
            match apply_edit(&content, &e.oldText, &e.newText, args.fuzzy) {
                Ok(next) => content = next,
                Err(msg) => {
                    return Err(ToolError(format!("edit {}: {msg}", args.path)));
                }
            }
            applied += 1;
        }
        std::fs::write(&args.path, &content)
            .map_err(|e| ToolError(format!("edit {}: write back: {e}", args.path)))?;
        let summary = format!(
            "applied {applied} edit(s) to {} ({} bytes)",
            args.path,
            content.len()
        );
        let diff = super::unified_diff(&args.path, &old, &content);
        if diff.is_empty() {
            Ok(summary)
        } else {
            Ok(format!("{summary}\n{diff}"))
        }
    }
}

fn trunc(s: &str) -> String {
    const N: usize = 120;
    if s.len() <= N {
        s.to_string()
    } else {
        format!("{}…", &s[..N])
    }
}

/// Locate the byte span of `old` in `content`. Exact, unique `oldText`
/// always wins (both modes). When `fuzzy` and no exact match exists, fall
/// back to an `apply_patch`-style line match: every `old` line must match a
/// consecutive file line whitespace-insensitively, and the (single) window
/// is the change. Leading whitespace of the first line and trailing
/// whitespace of the last line are excluded from the span so indentation
/// survives replacement.
pub(crate) fn find_edit_span(
    content: &str,
    old: &str,
    fuzzy: bool,
) -> Result<(usize, usize), String> {
    let count = content.matches(old).count();
    if count > 1 {
        return Err(format!(
            "oldText matches {count} times, must be unique: {:?}",
            trunc(old)
        ));
    }
    if count == 1 {
        let s = content.find(old).expect("count matched");
        return Ok((s, s + old.len()));
    }
    if fuzzy {
        return fuzzy_span(content, old);
    }
    Err(format!("oldText not found: {:?}", trunc(old)))
}

/// Apply one edit to `content` (see [`find_edit_span`]).
pub(crate) fn apply_edit(
    content: &str,
    old: &str,
    new: &str,
    fuzzy: bool,
) -> Result<String, String> {
    let (s, e) = find_edit_span(content, old, fuzzy)?;
    Ok(format!("{}{}{}", &content[..s], new, &content[e..]))
}

/// Whitespace-insensitive line form: trimmed, runs collapsed to single
/// spaces.
fn norm_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Per-line byte ranges (`end` is newline-exclusive) of `content`.
fn file_line_spans(content: &str) -> Vec<(&str, usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for line in content.split('\n') {
        let end = start + line.len();
        out.push((line, start, end));
        start = end + 1;
    }
    out
}

fn fuzzy_span(content: &str, old: &str) -> Result<(usize, usize), String> {
    let want: Vec<String> = old.lines().map(norm_line).collect();
    if want.is_empty() {
        return Err(format!("oldText not found: {:?}", trunc(old)));
    }
    let lines = file_line_spans(content);
    let norms: Vec<String> = lines.iter().map(|(t, _, _)| norm_line(t)).collect();
    let mut spans = Vec::new();
    if want.len() <= norms.len() {
        for i in 0..=(norms.len() - want.len()) {
            if norms[i..i + want.len()] == want[..] {
                let (first, fs, _) = lines[i];
                let (last, _, le) = lines[i + want.len() - 1];
                let start = fs + first.len().saturating_sub(first.trim_start().len());
                let end = le - last.len().saturating_sub(last.trim_end().len());
                spans.push((start, end));
            }
        }
    }
    match spans.len() {
        0 => Err(format!("oldText not found: {:?}", trunc(old))),
        1 => Ok(spans[0]),
        n => Err(format!(
            "oldText matches {n} times, must be unique: {:?}",
            trunc(old)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_whitespace_insensitively() {
        let out = apply_edit("foo  x\nbar\n", "foo x", "new", true).unwrap();
        assert_eq!(out, "new\nbar\n");
    }

    #[test]
    fn fuzzy_preserves_leading_indentation() {
        let out = apply_edit("  return 1\n", "return 1", "return 2", true).unwrap();
        assert_eq!(out, "  return 2\n");
    }

    #[test]
    fn fuzzy_matches_multiline_block_and_keeps_interior_indent() {
        // Model forgot the interior indentation; line-norm still matches.
        let content = "cfg = {\n  \"a\": 1,\n  \"b\": 2,\n}\n";
        let old = "\"a\": 1,\n\"b\": 2,";
        let new = "\"a\": 1,\n  \"b\": 3,";
        let out = apply_edit(content, old, new, true).unwrap();
        assert_eq!(out, "cfg = {\n  \"a\": 1,\n  \"b\": 3,\n}\n");
    }

    #[test]
    fn fuzzy_exact_substring_still_wins_when_unique() {
        let out = apply_edit("foo bar baz\n", "bar", "BAR", true).unwrap();
        assert_eq!(out, "foo BAR baz\n");
    }

    #[test]
    fn fuzzy_rejects_ambiguous_windows() {
        let err = apply_edit("a  b\na  b\n", "a b", "x", true).unwrap_err();
        assert!(err.contains("matches 2 times"), "{err}");
    }

    #[test]
    fn fuzzy_reports_not_found() {
        let err = apply_edit("x y\n", "p q", "z", true).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn non_fuzzy_keeps_exact_contract_on_whitespace_mismatch() {
        let err = apply_edit("foo  bar\n", "foo bar", "x", false).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn non_fuzzy_unique_exact_applies() {
        let out = apply_edit("foo bar\n", "foo", "FOO", false).unwrap();
        assert_eq!(out, "FOO bar\n");
    }

    #[test]
    fn edit_args_fuzzy_flag_defaults_false() {
        let a: EditArgs = serde_json::from_value(serde_json::json!({
            "path": "f", "edits": [{"oldText": "a", "newText": "b"}]
        }))
        .unwrap();
        assert!(!a.fuzzy, "missing fuzzy must default to false");
        let b: EditArgs = serde_json::from_value(serde_json::json!({
            "path": "f", "fuzzy": true, "edits": []
        }))
        .unwrap();
        assert!(b.fuzzy);
    }

    #[tokio::test]
    async fn edit_call_output_carries_diff_below_summary() {
        let dir = std::env::temp_dir().join(format!("rem-edit-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "foo bar\n").unwrap();

        let tool = EditTool;
        let mut ctx = ToolContext::new();
        let args: EditArgs = serde_json::from_value(serde_json::json!({
            "path": file.to_string_lossy(),
            "edits": [{"oldText": "foo", "newText": "FOO"}]
        }))
        .unwrap();
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert!(out.starts_with("applied 1 edit(s)"), "got: {out}");
        assert!(out.lines().any(|l| l == "-foo bar"), "got: {out}");
        assert!(out.lines().any(|l| l == "+FOO bar"), "got: {out}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
