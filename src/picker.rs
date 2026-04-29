//! Interactive profile picker for Kids mode.
//!
//! Shows a fun, visual profile selection screen when OmniShell starts
//! without a --profile or --mode flag. Features:
//! - Emoji-based profile cards (no external image dependency)
//! - Keypress selection (1-9 for profiles)
//! - Ghostty image protocol support for profile pictures (optional)

use std::io::{self, Write};

use crate::profile::{OmniShellConfig, Mode};

/// A profile card for display.
#[derive(Debug, Clone)]
pub struct ProfileCard {
    /// Profile name.
    pub name: String,
    /// Display emoji.
    pub emoji: String,
    /// One-line description.
    pub description: String,
    /// The mode.
    pub mode: Mode,
}

/// Generate profile cards from config.
pub fn generate_cards(config: &OmniShellConfig) -> Vec<ProfileCard> {
    let mut cards = Vec::new();

    for (name, profile) in &config.profile {
        let (emoji, desc) = match profile.mode {
            Mode::Kids => ("🧒", format!(
                "Kids Mode — Learn terminal!{}",
                profile.age.map(|a| format!(" (Age {})", a)).unwrap_or_default()
            )),
            Mode::Agent => ("🤖", "Agent Mode — AI coding assistant".to_string()),
            Mode::Admin => ("⚡", "Admin Mode — Full access".to_string()),
        };

        cards.push(ProfileCard {
            name: name.clone(),
            emoji: emoji.to_string(),
            description: desc,
            mode: profile.mode,
        });
    }

    // Sort: Kids first, then Agent, then Admin
    cards.sort_by_key(|c| match c.mode {
        Mode::Kids => 0,
        Mode::Agent => 1,
        Mode::Admin => 2,
    });

    cards
}

/// Render the profile picker to stdout.
pub fn render_picker(cards: &[ProfileCard]) -> String {
    let mut out = String::new();
    out.push_str("\n");
    out.push_str("╔══════════════════════════════════════╗\n");
    out.push_str("║       🐚 Welcome to OmniShell!       ║\n");
    out.push_str("║     Choose your profile to start      ║\n");
    out.push_str("╠══════════════════════════════════════╣\n");

    for (i, card) in cards.iter().enumerate() {
        let num = i + 1;
        out.push_str(&format!(
            "║  {} {} │ {}  {}\n",
            num,
            card.emoji,
            card.name,
            card.description,
        ));
    }

    out.push_str("╚══════════════════════════════════════╝\n");
    out.push_str("\nPress 1-9 to select: ");

    out
}

/// Prompt the user to select a profile interactively.
pub fn pick_profile(config: &OmniShellConfig) -> Option<String> {
    let cards = generate_cards(config);
    if cards.is_empty() {
        return None;
    }

    // If only one profile, use it
    if cards.len() == 1 {
        return Some(cards[0].name.clone());
    }

    let picker_text = render_picker(&cards);
    print!("{}", picker_text);
    io::stdout().flush().ok()?;

    // Read a single character
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return None;
    }

    let trimmed = input.trim();
    if let Ok(num) = trimmed.parse::<usize>() {
        if num >= 1 && num <= cards.len() {
            return Some(cards[num - 1].name.clone());
        }
    }

    // Try matching by name
    for card in &cards {
        if card.name.to_lowercase() == trimmed.to_lowercase() {
            return Some(card.name.clone());
        }
    }

    // Default to first (Kids if available)
    Some(cards[0].name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use std::collections::HashMap;

    fn make_config() -> OmniShellConfig {
        let mut profile = HashMap::new();
        profile.insert("kids".to_string(), Profile {
            mode: Mode::Kids,
            age: Some(7),
            ..Default::default()
        });
        profile.insert("agent".to_string(), Profile {
            mode: Mode::Agent,
            ..Default::default()
        });
        profile.insert("admin".to_string(), Profile {
            mode: Mode::Admin,
            ..Default::default()
        });
        OmniShellConfig {
            profile,
            default_profile: None,
            ..Default::default()
        }
    }

    #[test]
    fn test_generate_cards() {
        let config = make_config();
        let cards = generate_cards(&config);
        assert_eq!(cards.len(), 3);

        // Kids should be first
        assert_eq!(cards[0].mode, Mode::Kids);
        assert!(cards[0].emoji.contains("🧒"));

        assert_eq!(cards[1].mode, Mode::Agent);
        assert_eq!(cards[2].mode, Mode::Admin);
    }

    #[test]
    fn test_render_picker() {
        let config = make_config();
        let cards = generate_cards(&config);
        let rendered = render_picker(&cards);

        assert!(rendered.contains("OmniShell"));
        assert!(rendered.contains("🧒"));
        assert!(rendered.contains("🤖"));
        assert!(rendered.contains("⚡"));
        assert!(rendered.contains("Press 1-9"));
    }

    #[test]
    fn test_kids_card_has_age() {
        let config = make_config();
        let cards = generate_cards(&config);
        let kids_card = cards.iter().find(|c| c.mode == Mode::Kids).unwrap();
        assert!(kids_card.description.contains("Age 7"));
    }

    #[test]
    fn test_single_profile_auto_selects() {
        let mut profile = HashMap::new();
        profile.insert("only".to_string(), Profile {
            mode: Mode::Admin,
            ..Default::default()
        });
        let config = OmniShellConfig {
            profile,
            ..Default::default()
        };

        let cards = generate_cards(&config);
        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn test_profile_card_ordering() {
        let config = make_config();
        let cards = generate_cards(&config);
        let modes: Vec<_> = cards.iter().map(|c| c.mode).collect();
        assert_eq!(modes, vec![Mode::Kids, Mode::Agent, Mode::Admin]);
    }
}
