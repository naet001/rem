pub mod bash;
pub mod edit;
pub mod git;
pub mod glob_tool;
pub mod grep;
pub mod list_directory;
pub mod read;
pub mod web;
pub mod write;

pub use bash::BashTool;
pub use edit::EditTool;
pub use git::{GitDiffTool, GitStatusTool};
pub use glob_tool::GlobTool;
pub use grep::GrepTool;
pub use list_directory::ListDirectoryTool;
pub use read::ReadTool;
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
