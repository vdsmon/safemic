use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutConfig {
    #[serde(default)]
    pub modifiers: Vec<String>, // ["shift", "meta", "ctrl", "alt"]
    pub key: String, // "A", "M", "F13", etc.
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            modifiers: vec!["shift".to_string(), "meta".to_string()],
            key: "A".to_string(),
        }
    }
}

fn default_popup_duration_ms() -> u64 {
    1000
}

/// App appearance override. `System` clears the override so windows follow
/// the OS setting. The menu bar ignores app-level appearance, so the tray
/// icon keys off the OS theme regardless of this preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub mic_shortcut: ShortcutConfig,
    #[serde(default)]
    pub launch_at_login: bool,
    /// How long the on-screen popup bezel stays visible after a mute/unmute
    /// event. 0 hides the popup entirely; the menu bar icon still updates.
    #[serde(default = "default_popup_duration_ms")]
    pub popup_duration_ms: u64,
    #[serde(default)]
    pub theme: ThemePreference,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mic_shortcut: ShortcutConfig::default(),
            launch_at_login: false,
            popup_duration_ms: default_popup_duration_ms(),
            theme: ThemePreference::default(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from_file().unwrap_or_default()
    }

    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("safemic").join("settings.json"))
    }

    fn load_from_file() -> Option<Self> {
        let path = Self::config_path()?;
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Returns the last-modified time of the settings file, or None if it doesn't exist.
    pub fn mtime() -> Option<std::time::SystemTime> {
        Self::config_path()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
    }

    pub fn save(&self) -> Result<()> {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let data = serde_json::to_string_pretty(self)?;
            std::fs::write(path, data)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_shortcut() {
        let sc = ShortcutConfig::default();
        assert_eq!(sc.key, "A");
        assert!(sc.modifiers.contains(&"shift".to_string()));
        assert!(sc.modifiers.contains(&"meta".to_string()));
    }

    #[test]
    fn test_settings_json_round_trip() {
        let s = Settings::default();

        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.mic_shortcut.key, "A");
    }

    #[test]
    fn test_settings_json_missing_shortcut_modifiers() {
        let loaded: Settings = serde_json::from_str(
            r#"{
                "mic_shortcut": {
                    "key": "F13"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(loaded.mic_shortcut.key, "F13");
        assert!(loaded.mic_shortcut.modifiers.is_empty());
    }

    #[test]
    fn test_settings_save_and_load() {
        use std::fs;

        // Use a temp path for testing
        let tmp_dir = std::env::temp_dir().join("safemic-test-settings");
        let tmp_path = tmp_dir.join("settings.json");
        let _ = fs::remove_file(&tmp_path);
        let _ = fs::create_dir_all(&tmp_dir);

        let s = Settings {
            mic_shortcut: ShortcutConfig {
                modifiers: vec!["shift".to_string()],
                key: "M".to_string(),
            },
            launch_at_login: false,
            popup_duration_ms: 1000,
            theme: ThemePreference::default(),
        };

        let json = serde_json::to_string_pretty(&s).unwrap();
        fs::write(&tmp_path, &json).unwrap();

        let loaded: Settings =
            serde_json::from_str(&fs::read_to_string(&tmp_path).unwrap()).unwrap();
        assert_eq!(loaded.mic_shortcut.key, "M");
        assert_eq!(loaded.popup_duration_ms, 1000);

        let _ = fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_default_popup_duration_ms() {
        assert_eq!(Settings::default().popup_duration_ms, 1000);
    }

    #[test]
    fn test_settings_json_missing_popup_duration_defaults_to_1000() {
        let loaded: Settings = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(loaded.popup_duration_ms, 1000);
    }

    #[test]
    fn test_settings_json_popup_duration_zero_round_trip() {
        let loaded: Settings = serde_json::from_str(r#"{"popup_duration_ms": 0}"#).unwrap();
        assert_eq!(loaded.popup_duration_ms, 0);
        let json = serde_json::to_string(&loaded).unwrap();
        let round: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(round.popup_duration_ms, 0);
    }

    #[test]
    fn test_settings_json_missing_theme_defaults_to_system() {
        let loaded: Settings = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(loaded.theme, ThemePreference::System);
    }

    #[test]
    fn test_settings_json_theme_round_trip() {
        let loaded: Settings = serde_json::from_str(r#"{"theme": "dark"}"#).unwrap();
        assert_eq!(loaded.theme, ThemePreference::Dark);
        let json = serde_json::to_string(&loaded).unwrap();
        assert!(json.contains(r#""theme":"dark""#));
        let round: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(round.theme, ThemePreference::Dark);
    }
}
