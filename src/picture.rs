//! Profile picture support for OmniShell.
//!
//! Uses `viuer` for terminal image rendering and `image` for loading/processing.
//! Profile pictures are loaded from the XDG config directory and displayed
//! in the profile picker and prompt header.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// Profile picture configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilePicture {
    /// Path to the image file.
    pub path: PathBuf,
    /// Width in terminal columns (0 = auto).
    #[serde(default)]
    pub width: u32,
    /// Height in terminal rows (0 = auto).
    #[serde(default)]
    pub height: u32,
}

impl ProfilePicture {
    /// Create a new profile picture config.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            width: 0,
            height: 0,
        }
    }

    /// Create with explicit dimensions.
    pub fn with_size(path: impl Into<PathBuf>, width: u32, height: u32) -> Self {
        Self {
            path: path.into(),
            width,
            height,
        }
    }

    /// Get the default profile picture path for a mode.
    pub fn default_path(mode: Mode) -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("omnishell");

        let filename = match mode {
            Mode::Kids => "profile_kids.png",
            Mode::Agent => "profile_agent.png",
            Mode::Admin => "profile_admin.png",
        };

        config_dir.join("pictures").join(filename)
    }

    /// Check if the picture file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Render the picture to the terminal using viuer.
    /// Returns true if rendering succeeded.
    pub fn render(&self) -> bool {
        if !self.exists() {
            return false;
        }

        // Try to load and render the image
        let img = match image::open(&self.path) {
            Ok(img) => img,
            Err(_) => return false,
        };

        let config = viuer::Config {
            width: if self.width > 0 { Some(self.width) } else { None },
            height: if self.height > 0 { Some(self.height) } else { None },
            ..Default::default()
        };

        viuer::print(&img, &config).is_ok()
    }

    /// Get a fallback emoji for modes without a picture.
    pub fn fallback_emoji(mode: Mode) -> &'static str {
        match mode {
            Mode::Kids => "🧒",
            Mode::Agent => "🤖",
            Mode::Admin => "⚡",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_profile_picture() {
        let pic = ProfilePicture::new("/tmp/test.png");
        assert_eq!(pic.path, PathBuf::from("/tmp/test.png"));
        assert_eq!(pic.width, 0);
        assert_eq!(pic.height, 0);
    }

    #[test]
    fn test_with_size() {
        let pic = ProfilePicture::with_size("/tmp/test.png", 20, 10);
        assert_eq!(pic.width, 20);
        assert_eq!(pic.height, 10);
    }

    #[test]
    fn test_default_path_kids() {
        let path = ProfilePicture::default_path(Mode::Kids);
        assert!(path.to_str().unwrap().contains("profile_kids.png"));
    }

    #[test]
    fn test_default_path_agent() {
        let path = ProfilePicture::default_path(Mode::Agent);
        assert!(path.to_str().unwrap().contains("profile_agent.png"));
    }

    #[test]
    fn test_default_path_admin() {
        let path = ProfilePicture::default_path(Mode::Admin);
        assert!(path.to_str().unwrap().contains("profile_admin.png"));
    }

    #[test]
    fn test_exists_nonexistent() {
        let pic = ProfilePicture::new("/tmp/nonexistent_picture_12345.png");
        assert!(!pic.exists());
    }

    #[test]
    fn test_exists_with_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.png");
        std::fs::write(&file_path, "not a real png").unwrap();
        let pic = ProfilePicture::new(&file_path);
        assert!(pic.exists());
    }

    #[test]
    fn test_render_nonexistent_returns_false() {
        let pic = ProfilePicture::new("/tmp/nonexistent_12345.png");
        assert!(!pic.render());
    }

    #[test]
    fn test_fallback_emoji() {
        assert_eq!(ProfilePicture::fallback_emoji(Mode::Kids), "🧒");
        assert_eq!(ProfilePicture::fallback_emoji(Mode::Agent), "🤖");
        assert_eq!(ProfilePicture::fallback_emoji(Mode::Admin), "⚡");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let pic = ProfilePicture::with_size("/home/user/pic.png", 30, 15);
        let json = serde_json::to_string(&pic).unwrap();
        let parsed: ProfilePicture = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.path, pic.path);
        assert_eq!(parsed.width, 30);
        assert_eq!(parsed.height, 15);
    }

    #[test]
    fn test_default_path_is_in_config_dir() {
        for mode in [Mode::Kids, Mode::Agent, Mode::Admin] {
            let path = ProfilePicture::default_path(mode);
            assert!(
                path.to_str().unwrap().contains("omnishell"),
                "Default path should be in omnishell config dir"
            );
            assert!(
                path.to_str().unwrap().contains("pictures"),
                "Default path should be in pictures subdir"
            );
        }
    }
}
