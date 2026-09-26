//! Window close preference, kept with the other user config files.

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
struct UiPreferences {
    close_behavior: CloseBehavior,
}

pub fn load(root: &Path) -> CloseBehavior {
    std::fs::read_to_string(root.join(FILE_NAME))
        .ok()
        .and_then(|raw| toml::from_str::<UiPreferences>(&raw).ok())
        .unwrap_or_default()
        .close_behavior
}

pub fn save(root: &Path, behavior: CloseBehavior) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|err| err.to_string())?;
    let raw = toml::to_string(&UiPreferences {
        close_behavior: behavior,
    })
    .map_err(|err| err.to_string())?;
    std::fs::write(root.join(FILE_NAME), raw).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
