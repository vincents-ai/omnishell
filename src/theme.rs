//! Theme support for OmniShell.
//!
//! Per-profile themes that control PS1 prompt, colors, and visual style.
//! Themes are defined in config and selected based on the active profile.

use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// Color specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Rgb(u8, u8, u8),
    Default,
}

impl Color {
    /// Convert to ANSI escape code.
    pub fn to_ansi_fg(&self) -> String {
        match self {
            Color::Black => "30".to_string(),
            Color::Red => "31".to_string(),
            Color::Green => "32".to_string(),
            Color::Yellow => "33".to_string(),
            Color::Blue => "34".to_string(),
            Color::Magenta => "35".to_string(),
            Color::Cyan => "36".to_string(),
            Color::White => "37".to_string(),
            Color::BrightBlack => "90".to_string(),
            Color::BrightRed => "91".to_string(),
            Color::BrightGreen => "92".to_string(),
            Color::BrightYellow => "93".to_string(),
            Color::BrightBlue => "94".to_string(),
            Color::BrightMagenta => "95".to_string(),
            Color::BrightCyan => "96".to_string(),
            Color::BrightWhite => "97".to_string(),
            Color::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
            Color::Default => "39".to_string(),
        }
    }

    /// Apply as foreground color.
    pub fn fg(&self) -> String {
        format!("\x1b[{}m", self.to_ansi_fg())
    }
}

/// A theme definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    /// Theme name.
    pub name: String,
    /// Primary accent color.
    pub primary: Color,
    /// Secondary accent color.
    pub secondary: Color,
    /// Error/danger color.
    pub error: Color,
    /// Success color.
    pub success: Color,
    /// PS1 prompt template.
    /// Supports: {user}, {host}, {cwd}, {mode}, {git_branch}, {emoji}
    #[serde(default = "default_prompt")]
    pub prompt: String,
    /// Mode emoji.
    #[serde(default)]
    pub emoji: Option<String>,
}

fn default_prompt() -> String {
    "{emoji} {user}@{host}:{cwd}$ ".to_string()
}

impl Theme {
    /// Kids theme: friendly, colorful, emoji-heavy.
    pub fn kids() -> Self {
        Self {
            name: "kids".to_string(),
            primary: Color::Cyan,
            secondary: Color::BrightYellow,
            error: Color::BrightRed,
            success: Color::BrightGreen,
            prompt: "🐚 {emoji} {cwd}> ".to_string(),
            emoji: Some("🧒".to_string()),
        }
    }

    /// Agent theme: minimal, structured.
    pub fn agent() -> Self {
        Self {
            name: "agent".to_string(),
            primary: Color::BrightBlue,
            secondary: Color::BrightCyan,
            error: Color::Red,
            success: Color::Green,
            prompt: "[{mode}] {user}:{cwd}$ ".to_string(),
            emoji: Some("🤖".to_string()),
        }
    }

    /// Admin theme: classic terminal.
    pub fn admin() -> Self {
        Self {
            name: "admin".to_string(),
            primary: Color::BrightGreen,
            secondary: Color::BrightCyan,
            error: Color::BrightRed,
            success: Color::Green,
            prompt: "{user}@{host}:{cwd}$ ".to_string(),
            emoji: Some("⚡".to_string()),
        }
    }

    /// Get theme for mode.
    pub fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Kids => Self::kids(),
            Mode::Agent => Self::agent(),
            Mode::Admin => Self::admin(),
        }
    }

    /// Render the prompt with variables filled in.
    pub fn render_prompt(&self, user: &str, host: &str, cwd: &str, git_branch: Option<&str>) -> String {
        let emoji = self.emoji.as_deref().unwrap_or("");
        let mode = match self.name.as_str() {
            "kids" => "kids",
            "agent" => "agent",
            _ => "admin",
        };

        self.prompt
            .replace("{user}", user)
            .replace("{host}", host)
            .replace("{cwd}", cwd)
            .replace("{mode}", mode)
            .replace("{git_branch}", git_branch.unwrap_or(""))
            .replace("{emoji}", emoji)
    }

    /// Colorize text with the primary color.
    pub fn primary(&self, text: &str) -> String {
        format!("{}{}\x1b[0m", self.primary.fg(), text)
    }

    /// Colorize text with the error color.
    pub fn error(&self, text: &str) -> String {
        format!("{}{}\x1b[0m", self.error.fg(), text)
    }

    /// Colorize text with the success color.
    pub fn success(&self, text: &str) -> String {
        format!("{}{}\x1b[0m", self.success.fg(), text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kids_theme() {
        let theme = Theme::kids();
        assert_eq!(theme.name, "kids");
        assert_eq!(theme.primary, Color::Cyan);
        assert!(theme.prompt.contains("🐚"));
    }

    #[test]
    fn test_agent_theme() {
        let theme = Theme::agent();
        assert_eq!(theme.name, "agent");
        assert_eq!(theme.primary, Color::BrightBlue);
        assert!(theme.prompt.contains("{mode}"));
    }

    #[test]
    fn test_admin_theme() {
        let theme = Theme::admin();
        assert_eq!(theme.name, "admin");
        assert_eq!(theme.primary, Color::BrightGreen);
        assert!(theme.prompt.contains("{user}@{host}"));
    }

    #[test]
    fn test_for_mode() {
        assert_eq!(Theme::for_mode(Mode::Kids).name, "kids");
        assert_eq!(Theme::for_mode(Mode::Agent).name, "agent");
        assert_eq!(Theme::for_mode(Mode::Admin).name, "admin");
    }

    #[test]
    fn test_render_prompt() {
        let theme = Theme::admin();
        let rendered = theme.render_prompt("user", "host", "/home/user", Some("main"));
        assert!(rendered.contains("user@host:/home/user$"));
    }

    #[test]
    fn test_render_prompt_no_git_branch() {
        let theme = Theme::admin();
        let rendered = theme.render_prompt("user", "host", "/tmp", None);
        assert!(rendered.contains("user@host:/tmp$"));
    }

    #[test]
    fn test_color_ansi_codes() {
        assert_eq!(Color::Red.to_ansi_fg(), "31");
        assert_eq!(Color::BrightGreen.to_ansi_fg(), "92");
        assert_eq!(Color::Default.to_ansi_fg(), "39");
        assert_eq!(Color::Rgb(255, 128, 0).to_ansi_fg(), "38;2;255;128;0");
    }

    #[test]
    fn test_color_fg() {
        let colored = Color::Green.fg();
        assert!(colored.contains("\x1b[32m"));
    }

    #[test]
    fn test_theme_colorize() {
        let theme = Theme::admin();
        let primary = theme.primary("hello");
        assert!(primary.contains("hello"));
        assert!(primary.contains("\x1b["));

        let error = theme.error("fail");
        assert!(error.contains("fail"));

        let success = theme.success("ok");
        assert!(success.contains("ok"));
    }

    #[test]
    fn test_theme_serialization_roundtrip() {
        let theme = Theme::kids();
        let json = serde_json::to_string(&theme).unwrap();
        let parsed: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "kids");
        assert_eq!(parsed.primary, Color::Cyan);
    }

    #[test]
    fn test_color_serialization() {
        let json = serde_json::to_string(&Color::Red).unwrap();
        assert_eq!(json, "\"red\"");

        let parsed: Color = serde_json::from_str("\"brightblue\"").unwrap();
        assert_eq!(parsed, Color::BrightBlue);

        let rgb = serde_json::to_string(&Color::Rgb(255, 0, 128)).unwrap();
        assert!(rgb.contains("255"));
    }
}
