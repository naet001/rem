//! Hard-coded permission modes for the interactive session.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    Plan,
    #[default]
    Manual,
    Auto,
    Edit,
    Yolo,
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Plan, Mode::Manual, Mode::Auto, Mode::Edit, Mode::Yolo];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Manual => "manual",
            Self::Auto => "auto",
            Self::Edit => "edit",
            Self::Yolo => "yolo",
        }
    }

    /// Parse a `--mode <name>` value (grilled P0-#2: the single headless
    /// mode flag; no `--yolo`/`--plan` shorthands). Case-insensitive.
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "plan" => Some(Self::Plan),
            "manual" => Some(Self::Manual),
            "auto" => Some(Self::Auto),
            "edit" => Some(Self::Edit),
            "yolo" => Some(Self::Yolo),
            _ => None,
        }
    }
}
