use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

use super::ToolError;

#[derive(Debug, Deserialize)]
pub struct ReadArgs {
    pub path: String,
    #[serde(default, deserialize_with = "super::opt_usize_lenient")]
    pub offset: Option<usize>,
    #[serde(default, deserialize_with = "super::opt_usize_lenient")]
    pub limit: Option<usize>,
}

#[derive(Debug)]
pub struct ReadTool;

impl Tool for ReadTool {
    const NAME: &'static str = "read";
    type Args = ReadArgs;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Read a file to string. Optional line `offset` (0-based) and `limit`. Errors if the file cannot be read.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to read" },
                "offset": { "type": "integer", "description": "First line to return (0-based). Omit for start of file." },
                "limit": { "type": "integer", "description": "Max lines to return. Omit for the whole file." }
            },
            "required": ["path"]
        })
    }

    async fn call(&self, _ctx: &mut ToolContext, args: Self::Args) -> Result<String, ToolError> {
        let content = std::fs::read_to_string(&args.path)
            .map_err(|e| ToolError(format!("read {}: {e}", args.path)))?;
        match (args.offset, args.limit) {
            (None, None) => Ok(content),
            (offset, limit) => {
                let start = offset.unwrap_or(0);
                let lines: Vec<&str> = content.lines().collect();
                let end = match limit {
                    Some(n) => (start + n).min(lines.len()),
                    None => lines.len(),
                };
                let start = start.min(lines.len());
                Ok(lines[start..end].join("\n"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_float_offsets_like_120_dot_0() {
        // Regression: models emit `{"offset": 120.0}` for `"type": "number"`
        // schemas, which failed with
        // `invalid type: floating point 120.0, expected usize`.
        let args: ReadArgs = serde_json::from_value(
            serde_json::json!({"path": "x", "offset": 120.0, "limit": 50.0}),
        )
        .expect("float 120.0 should parse as usize 120");
        assert_eq!(args.offset, Some(120));
        assert_eq!(args.limit, Some(50));
    }

    #[test]
    fn parses_ints_and_missing_fields() {
        let args: ReadArgs =
            serde_json::from_value(serde_json::json!({"path": "x", "offset": 5, "limit": 10}))
                .unwrap();
        assert_eq!(args.offset, Some(5));
        assert_eq!(args.limit, Some(10));

        let args: ReadArgs = serde_json::from_value(serde_json::json!({"path": "x"})).unwrap();
        assert_eq!(args.offset, None);
        assert_eq!(args.limit, None);
    }
}
