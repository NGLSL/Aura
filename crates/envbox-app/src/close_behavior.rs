//! Window close preference and pane widths, kept with the other user config files.

use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE_NAME: &str = "ui.toml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    #[default]
    Ask,
    Tray,
    Exit,
}

impl CloseBehavior {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "每次询问",
            Self::Tray => "最小化到系统托盘",
            Self::Exit => "退出 Aura",
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct UiPreferences {
    close_behavior: CloseBehavior,
    pane_ratios: PaneRatios,
}

/// List widths as a fraction of the area to the right of the navigation bar.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneRatios {
    pub apps: f32,
    pub profiles: f32,
}

impl Default for PaneRatios {
    fn default() -> Self {
        Self {
            apps: 0.60,
            profiles: 0.60,
        }
    }
}

impl PaneRatios {
    pub fn normalize(&mut self) {
        let defaults = Self::default();
        self.apps = normalized_ratio(self.apps, 0.35, defaults.apps);
        self.profiles = normalized_ratio(self.profiles, 0.32, defaults.profiles);
    }
}

fn normalized_ratio(value: f32, minimum: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, 0.75)
    } else {
        default
    }
}

fn load_preferences(root: &Path) -> UiPreferences {
    std::fs::read_to_string(root.join(FILE_NAME))
        .ok()
        .and_then(|raw| toml::from_str::<UiPreferences>(&raw).ok())
        .unwrap_or_default()
}

pub fn load(root: &Path) -> CloseBehavior {
    load_preferences(root).close_behavior
}

pub fn load_pane_ratios(root: &Path) -> PaneRatios {
    let mut ratios = load_preferences(root).pane_ratios;
    ratios.normalize();
    ratios
}

pub fn save(root: &Path, behavior: CloseBehavior) -> Result<(), String> {
    let mut preferences = load_preferences(root);
    preferences.close_behavior = behavior;
    save_preferences(root, &preferences)
}

pub fn save_pane_ratios(root: &Path, mut ratios: PaneRatios) -> Result<(), String> {
    ratios.normalize();
    let mut preferences = load_preferences(root);
    preferences.pane_ratios = ratios;
    save_preferences(root, &preferences)
}

fn save_preferences(root: &Path, preferences: &UiPreferences) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|err| err.to_string())?;
    let raw = toml::to_string(preferences).map_err(|err| err.to_string())?;
    std::fs::write(root.join(FILE_NAME), raw).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_close_preference_uses_default_widths_and_invalid_widths_are_bounded() {
        let legacy: UiPreferences = toml::from_str("close_behavior = 'tray'").unwrap();
        assert_eq!(legacy.close_behavior, CloseBehavior::Tray);
        assert_eq!(legacy.pane_ratios.apps, 0.60);
        assert_eq!(legacy.pane_ratios.profiles, 0.60);

        let mut preferences: UiPreferences =
            toml::from_str("[pane_ratios]\napps = nan\nprofiles = 9.0\n").unwrap();
        preferences.pane_ratios.normalize();
        assert_eq!(preferences.pane_ratios.apps, 0.60);
        assert_eq!(preferences.pane_ratios.profiles, 0.75);
    }

    #[test]
    fn remembered_choice_survives_reload_and_can_be_reset() {
        let root = std::env::temp_dir().join(format!("envbox-ui-test-{}", uuid::Uuid::new_v4()));
        assert_eq!(load(&root), CloseBehavior::Ask);
        save(&root, CloseBehavior::Tray).unwrap();
        assert_eq!(load(&root), CloseBehavior::Tray);
        save(&root, CloseBehavior::Exit).unwrap();
        assert_eq!(load(&root), CloseBehavior::Exit);
        save(&root, CloseBehavior::Ask).unwrap();
        assert_eq!(load(&root), CloseBehavior::Ask);
        std::fs::remove_file(root.join(FILE_NAME)).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
