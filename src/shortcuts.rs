use crate::settings::{Settings, ShortcutConfig};
use anyhow::{Context, Result};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyManager,
};

/// macOS-reserved combos we refuse to register, with a short user-facing
/// label. The Modifiers value must match exactly (no extra bits, no missing
/// bits) — we don't honor superset matches. Key strings use the same labels
/// that `code_from_str` accepts.
pub const RESERVED_COMBOS: &[(Modifiers, &str, &str)] = &[
    (Modifiers::META, "Space", "Spotlight"),
    (
        Modifiers::META.union(Modifiers::ALT),
        "Space",
        "Finder search",
    ),
    (Modifiers::CONTROL, "Space", "Input source switch"),
    (Modifiers::META, "Tab", "App switcher"),
    (Modifiers::META, "Backquote", "Window switcher"),
    (Modifiers::META, "Q", "Quit application"),
    (Modifiers::META, "W", "Close window"),
    (Modifiers::META, "H", "Hide application"),
    (Modifiers::META, "M", "Minimize window"),
    (Modifiers::META, "Comma", "Preferences"),
    (
        Modifiers::META.union(Modifiers::ALT),
        "Escape",
        "Force Quit",
    ),
    (
        Modifiers::META.union(Modifiers::SHIFT),
        "Period",
        "Toggle hidden files",
    ),
    (
        Modifiers::META.union(Modifiers::CONTROL),
        "F",
        "Toggle fullscreen",
    ),
    (
        Modifiers::CONTROL.union(Modifiers::META),
        "Q",
        "Lock screen",
    ),
    (
        Modifiers::META.union(Modifiers::SHIFT),
        "3",
        "Screenshot: full screen",
    ),
    (
        Modifiers::META.union(Modifiers::SHIFT),
        "4",
        "Screenshot: selection",
    ),
    (
        Modifiers::META.union(Modifiers::SHIFT),
        "5",
        "Screenshot tools",
    ),
    (
        Modifiers::META.union(Modifiers::SHIFT),
        "6",
        "Screenshot to clipboard",
    ),
    (Modifiers::empty(), "F11", "Show desktop"),
    (Modifiers::empty(), "F12", "Dashboard"),
    (Modifiers::CONTROL, "ArrowUp", "Mission Control"),
    (Modifiers::CONTROL, "ArrowDown", "Application windows"),
    (Modifiers::CONTROL, "ArrowLeft", "Move left a space"),
    (Modifiers::CONTROL, "ArrowRight", "Move right a space"),
];

#[derive(Debug)]
pub enum ShortcutConflict {
    ReservedByMacOS(&'static str),
    InvalidKey(String),
}

impl std::fmt::Display for ShortcutConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShortcutConflict::ReservedByMacOS(name) => {
                write!(f, "\u{26A0} Used by macOS: {}", name)
            }
            ShortcutConflict::InvalidKey(key) => write!(f, "\u{26A0} Invalid key: {}", key),
        }
    }
}

#[allow(dead_code)]
pub struct Shortcuts {
    hotkeys_manager: GlobalHotKeyManager,
    pub mic_hotkey: HotKey,
}

fn modifiers_from_config(config: &ShortcutConfig) -> Modifiers {
    let mut mods = Modifiers::empty();
    for m in &config.modifiers {
        match m.as_str() {
            "shift" => mods |= Modifiers::SHIFT,
            "meta" | "cmd" | "command" => mods |= Modifiers::META,
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" => mods |= Modifiers::ALT,
            _ => {}
        }
    }
    mods
}

fn code_from_str(key: &str) -> Option<Code> {
    key.parse::<HotKey>().ok().map(|hotkey| hotkey.key)
}

fn hotkey_from_config(config: &ShortcutConfig) -> Option<HotKey> {
    let mods = modifiers_from_config(config);
    let code = code_from_str(&config.key)?;
    Some(HotKey::new(Some(mods), code))
}

/// Validate a candidate shortcut against the macOS reserved list and key
/// sanity. Returns `Ok(())` if the combo parses and isn't reserved. A live
/// register-collision can only be surfaced by the real `Shortcuts::reload`
/// path — see the rollback path in `event_loop`.
pub fn validate_shortcut(config: &ShortcutConfig) -> std::result::Result<(), ShortcutConflict> {
    let mods = modifiers_from_config(config);
    let candidate_code = code_from_str(&config.key)
        .ok_or_else(|| ShortcutConflict::InvalidKey(config.key.clone()))?;

    for (reserved_mods, reserved_key, label) in RESERVED_COMBOS {
        if *reserved_mods == mods && code_from_str(reserved_key) == Some(candidate_code) {
            return Err(ShortcutConflict::ReservedByMacOS(label));
        }
    }

    Ok(())
}

impl Shortcuts {
    pub fn new(settings: &Settings) -> Result<Self> {
        let hotkeys_manager = GlobalHotKeyManager::new().unwrap();

        let mic_hotkey = hotkey_from_config(&settings.mic_shortcut)
            .ok_or_else(|| anyhow::anyhow!("invalid key '{}'", settings.mic_shortcut.key))?;

        hotkeys_manager
            .register(mic_hotkey)
            .context("Failed to register mic hotkey")?;

        Ok(Self {
            hotkeys_manager,
            mic_hotkey,
        })
    }

    /// Temporarily unregister the hotkey so the shortcut recorder can capture
    /// the currently-assigned combo (a registered hotkey consumes it before
    /// any NSView sees the keyDown). Idempotent.
    pub fn suspend(&mut self) {
        let _ = self.hotkeys_manager.unregister(self.mic_hotkey);
    }

    /// Re-register after `suspend`. A subsequent `reload` handles the case
    /// where the shortcut changed while suspended.
    pub fn resume(&mut self) -> Result<()> {
        self.hotkeys_manager
            .register(self.mic_hotkey)
            .context("Failed to re-register mic hotkey")
    }

    /// Unregister the current hotkeys and register new ones from updated settings.
    pub fn reload(&mut self, settings: &Settings) -> Result<()> {
        let _ = self.hotkeys_manager.unregister(self.mic_hotkey);

        self.mic_hotkey = hotkey_from_config(&settings.mic_shortcut)
            .ok_or_else(|| anyhow::anyhow!("invalid key '{}'", settings.mic_shortcut.key))?;

        self.hotkeys_manager
            .register(self.mic_hotkey)
            .context("Failed to register mic hotkey")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ShortcutConfig;

    #[test]
    fn test_code_from_str_uppercase() {
        assert!(matches!(code_from_str("A"), Some(Code::KeyA)));
        assert!(matches!(code_from_str("V"), Some(Code::KeyV)));
    }

    #[test]
    fn test_code_from_str_lowercase() {
        assert!(matches!(code_from_str("a"), Some(Code::KeyA)));
        assert!(matches!(code_from_str("v"), Some(Code::KeyV)));
    }

    #[test]
    fn test_code_from_str_function_keys() {
        assert!(matches!(code_from_str("F1"), Some(Code::F1)));
        assert!(matches!(code_from_str("F13"), Some(Code::F13)));
        assert!(matches!(code_from_str("F24"), Some(Code::F24)));
    }

    #[test]
    fn test_code_from_str_invalid_returns_none() {
        assert!(code_from_str("NotAKey").is_none());
        assert!(code_from_str("").is_none());
    }

    #[test]
    fn test_reserved_combo_keys_parse() {
        for (_, key, label) in RESERVED_COMBOS {
            assert!(
                code_from_str(key).is_some(),
                "RESERVED_COMBOS entry '{}' has unparseable key '{}'",
                label,
                key,
            );
        }
    }

    #[test]
    fn test_hotkey_from_config_no_modifiers() {
        let config = ShortcutConfig {
            modifiers: vec![],
            key: "F13".to_string(),
        };
        let mods = modifiers_from_config(&config);
        assert!(mods.is_empty());
    }

    #[test]
    fn test_modifiers_from_config() {
        let config = ShortcutConfig {
            modifiers: vec!["shift".to_string(), "meta".to_string()],
            key: "A".to_string(),
        };
        let mods = modifiers_from_config(&config);
        assert!(mods.contains(Modifiers::SHIFT));
        assert!(mods.contains(Modifiers::META));
        assert!(!mods.contains(Modifiers::CONTROL));
    }

    #[test]
    fn test_modifiers_from_config_all() {
        let config = ShortcutConfig {
            modifiers: vec![
                "shift".to_string(),
                "ctrl".to_string(),
                "alt".to_string(),
                "meta".to_string(),
            ],
            key: "A".to_string(),
        };
        let mods = modifiers_from_config(&config);
        assert!(mods.contains(Modifiers::SHIFT));
        assert!(mods.contains(Modifiers::CONTROL));
        assert!(mods.contains(Modifiers::ALT));
        assert!(mods.contains(Modifiers::META));
    }

    #[test]
    fn test_validate_rejects_cmd_space() {
        let config = ShortcutConfig {
            modifiers: vec!["meta".to_string()],
            key: "Space".to_string(),
        };
        let result = validate_shortcut(&config);
        match result {
            Err(ShortcutConflict::ReservedByMacOS(name)) => {
                assert_eq!(name, "Spotlight");
            }
            other => panic!("expected ReservedByMacOS(Spotlight), got {:?}", other),
        }
    }

    #[test]
    fn test_validate_accepts_shift_cmd_m() {
        // Shift+Cmd+M is not on the reserved list (Cmd+M alone is, but the
        // shift differentiates it). With validate_shortcut no longer doing a
        // live-register probe, this must succeed.
        let config = ShortcutConfig {
            modifiers: vec!["shift".to_string(), "meta".to_string()],
            key: "M".to_string(),
        };
        assert!(validate_shortcut(&config).is_ok());
    }

    #[test]
    fn test_validate_rejects_invalid_key() {
        let config = ShortcutConfig {
            modifiers: vec!["meta".to_string()],
            key: "NotAKey".to_string(),
        };
        match validate_shortcut(&config) {
            Err(ShortcutConflict::InvalidKey(k)) => assert_eq!(k, "NotAKey"),
            other => panic!("expected InvalidKey, got {:?}", other),
        }
    }

    #[test]
    fn test_reserved_list_has_spotlight() {
        let found = RESERVED_COMBOS
            .iter()
            .any(|(_, _, name)| *name == "Spotlight");
        assert!(found, "Spotlight entry missing from RESERVED_COMBOS");
    }
}
