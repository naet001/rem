use std::path::Path;

/// Compute the human-readable patch a Mutate call (`edit` / `write`) would
/// apply, for the diff-accept flow: shown to the Approver before the tool
/// lands, rendered as `+`/`-` tinted rows in the approval modal.
///
/// Returns `None` when the tool is not Mutate, the args don't parse, the
/// target can't be read, or (for `edit`) the edits don't apply exactly —
/// the approval then degrades to the plain args preview it has today.
pub fn propose_patch(
    tool_name: &str,
    args: &serde_json::Value,
    project_root: &Path,
) -> Option<String> {
    match tool_name {
        "edit" => propose_edit_patch(args, project_root),
        "write" => propose_write_patch(args, project_root),
        _ => None,
    }
}

/// Resolve the target path: absolute paths run as-is, relative paths try
/// the raw path first (process cwd, matching the tools) then project root.
fn resolve(path: &str, project_root: &Path) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    if p.exists() {
        return Some(p.to_path_buf());
    }
    let joined = project_root.join(p);
    joined.exists().then_some(joined)
}

fn propose_edit_patch(args: &serde_json::Value, project_root: &Path) -> Option<String> {
    let path = args.get("path")?.as_str()?;
    let edits = args.get("edits")?.as_array()?;
    if edits.is_empty() {
        return None;
    }
    // Echo the tool's matching mode: `fuzzy:true` relaxes to
    // whitespace-insensitive lines (see edit::find_edit_span).
    let fuzzy = args.get("fuzzy").and_then(|v| v.as_bool()).unwrap_or(false);
    let resolved = resolve(path, project_root)?;
    let mut content = std::fs::read_to_string(&resolved).ok()?;
    let mut body = String::new();
    for e in edits {
        let old = e.get("oldText")?.as_str()?;
        let new = e.get("newText")?.as_str()?;
        // Same contract as the tool: an inapplicable edit has no patch.
        let (s, end) = super::edit::find_edit_span(&content, old, fuzzy).ok()?;
        for line in content[s..end].lines() {
            body.push('-');
            body.push_str(line);
            body.push('\n');
        }
        for line in new.lines() {
            body.push('+');
            body.push_str(line);
            body.push('\n');
        }
        content = format!("{}{}{}", &content[..s], new, &content[end..]);
    }
    Some(body)
}

fn propose_write_patch(args: &serde_json::Value, project_root: &Path) -> Option<String> {
    let path = args.get("path")?.as_str()?;
    let content = args.get("content")?.as_str()?;
    // Existing content removed, new content added; a new file is all `+`.
    let old = resolve(path, project_root)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut body = String::new();
    for line in old.lines() {
        body.push('-');
        body.push_str(line);
        body.push('\n');
    }
    for line in content.lines() {
        body.push('+');
        body.push_str(line);
        body.push('\n');
    }
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test dir: parallel tests must never share (or wipe) a fixture.
    fn root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rem-patch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn edit_args(path: &str, old: &str, new: &str) -> serde_json::Value {
        serde_json::json!({
            "path": path,
            "edits": [{ "oldText": old, "newText": new }]
        })
    }

    #[test]
    fn edit_patch_lists_removed_then_added_lines() {
        let dir = root("edit-lists");
        std::fs::write(dir.join("f.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        let args = edit_args("f.rs", "fn a() {}", "fn a(x: u8) {}");
        let patch = propose_patch("edit", &args, &dir).expect("patch proposed");
        assert!(patch.contains("-fn a() {}"), "got: {patch}");
        assert!(patch.contains("+fn a(x: u8) {}"), "got: {patch}");
        // The untouched line is not part of the proposal.
        assert!(!patch.contains("fn b"), "got: {patch}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_patch_multiline_lists_every_line() {
        let dir = root("edit-multiline");
        std::fs::write(dir.join("f.txt"), "one\ntwo\nthree\n").unwrap();
        let args = edit_args("f.txt", "one\ntwo", "uno\ndos");
        let patch = propose_patch("edit", &args, &dir).expect("patch proposed");
        assert_eq!(patch, "-one\n-two\n+uno\n+dos\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_patch_none_when_edits_do_not_apply() {
        let dir = root("edit-none");
        std::fs::write(dir.join("f.txt"), "only this\n").unwrap();
        // Missing text: the tool would fail, so there is no patch to accept.
        assert!(propose_patch("edit", &edit_args("f.txt", "nope", "x"), &dir).is_none());
        // Ambiguous (matches twice): same contract.
        std::fs::write(dir.join("g.txt"), "dup\ndup\n").unwrap();
        assert!(propose_patch("edit", &edit_args("g.txt", "dup", "x"), &dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_patch_shows_existing_removed_and_new_added() {
        let dir = root("write-existing");
        std::fs::write(dir.join("f.txt"), "old line\n").unwrap();
        let args = serde_json::json!({ "path": "f.txt", "content": "new line\nmore\n" });
        let patch = propose_patch("write", &args, &dir).expect("patch proposed");
        assert_eq!(patch, "-old line\n+new line\n+more\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_patch_for_new_file_is_all_additions() {
        let dir = root("write-new");
        let args = serde_json::json!({ "path": "fresh.txt", "content": "hello\n" });
        let patch = propose_patch("write", &args, &dir).expect("patch proposed");
        assert_eq!(patch, "+hello\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_patch_fuzzy_whitespace_insensitive_preview() {
        let dir = root("patch-fuzzy");
        std::fs::write(dir.join("f.txt"), "  alpha  beta\n").unwrap();
        let args = serde_json::json!({
            "path": "f.txt",
            "fuzzy": true,
            "edits": [{ "oldText": "alpha beta", "newText": "gamma" }]
        });
        let patch = propose_patch("edit", &args, &dir).expect("patch proposed");
        // Removed span is `alpha  beta` (matching is whitespace-insensitive);
        // the leading indentation is not part of the change.
        assert_eq!(patch, "-alpha  beta\n+gamma\n");
        // Same call without fuzzy: exact match fails, so no patch at all.
        let mut strict = args.clone();
        strict["fuzzy"] = serde_json::Value::Bool(false);
        assert!(propose_patch("edit", &strict, &dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_mutate_tools_get_no_patch() {
        let dir = root("non-mutate");
        let args = serde_json::json!({ "command": "rm -rf /" });
        assert!(propose_patch("bash", &args, &dir).is_none());
        assert!(propose_patch("read", &args, &dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
