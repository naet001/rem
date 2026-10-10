use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

use super::ToolError;

#[derive(Debug, Deserialize)]
pub struct WriteArgs {
    pub path: String,
    pub content: String,
}

#[derive(Debug)]
pub struct WriteTool;

impl Tool for WriteTool {
    const NAME: &'static str = "write";
    type Args = WriteArgs;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Create or overwrite a file with the given content. Parent directories are created as needed.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path of the file to write" },
                "content": { "type": "string", "description": "Full content to write to the file" }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(&self, _ctx: &mut ToolContext, args: Self::Args) -> Result<String, ToolError> {
        // Snapshot first: the transcript renders the old→new unified diff.
        let old = std::fs::read_to_string(&args.path).unwrap_or_default();
        if let Some(parent) = std::path::Path::new(&args.path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| ToolError(format!("write {}: create parent dirs: {e}", args.path)))?;
        }
        std::fs::write(&args.path, &args.content)
            .map_err(|e| ToolError(format!("write {}: {e}", args.path)))?;
        let summary = format!("wrote {} ({} bytes)", args.path, args.content.len());
        let diff = super::unified_diff(&args.path, &old, &args.content);
        if diff.is_empty() {
            Ok(summary)
        } else {
            Ok(format!("{summary}\n{diff}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_output_carries_summary_plus_unified_diff() {
        let dir = std::env::temp_dir().join(format!("rem-write-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "a\nb\n").unwrap();

        let tool = WriteTool;
        let mut ctx = ToolContext::new();
        let out = tool
            .call(
                &mut ctx,
                WriteArgs {
                    path: file.to_string_lossy().to_string(),
                    content: "a\nB\n".to_string(),
                },
            )
            .await
            .unwrap();
        assert!(out.starts_with("wrote "), "got: {out}");
        assert!(out.lines().any(|l| l == "-b"), "got: {out}");
        assert!(out.lines().any(|l| l == "+B"), "got: {out}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_identical_content_stays_summary_only() {
        let dir = std::env::temp_dir().join(format!("rem-write-same-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "same\n").unwrap();

        let tool = WriteTool;
        let mut ctx = ToolContext::new();
        let out = tool
            .call(
                &mut ctx,
                WriteArgs {
                    path: file.to_string_lossy().to_string(),
                    content: "same\n".to_string(),
                },
            )
            .await
            .unwrap();
        assert!(!out.contains("@@"), "got: {out}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
