//! Windows notification-area icon while the main window is hidden.

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayAction {
    Restore,
    Exit,
}

pub struct TrayState {
    _icon: TrayIcon,
    restore_id: MenuId,
    exit_id: MenuId,
}

impl TrayState {
    pub fn new() -> Result<Self, String> {
        let image = image::load_from_memory(include_bytes!("../../../icons/256x256.png"))
            .map_err(|err| err.to_string())?
            .to_rgba8();
        let icon = tray_icon::Icon::from_rgba(image.into_raw(), 256, 256)
            .map_err(|err| err.to_string())?;
        let menu = Menu::new();
        let restore = MenuItem::new("打开 Aura", true, None);
        let exit = MenuItem::new("退出 Aura", true, None);
        menu.append(&restore).map_err(|err| err.to_string())?;
        menu.append(&exit).map_err(|err| err.to_string())?;
        let restore_id = restore.id().clone();
        let exit_id = exit.id().clone();
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("Aura")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Self {
            _icon: tray,
            restore_id,
            exit_id,
        })
    }

    pub fn poll(&self) -> Option<TrayAction> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.restore_id {
                return Some(TrayAction::Restore);
            }
            if event.id == self.exit_id {
                return Some(TrayAction::Exit);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if event.id() != self._icon.id() {
                continue;
            }
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                return Some(TrayAction::Restore);
            }
        }
        None
    }
}
