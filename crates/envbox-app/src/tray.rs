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

fn tray_image() -> Result<image::RgbaImage, String> {
    image::load_from_memory(include_bytes!("../../../icons/256x256.png"))
        .map(|image| crate::app_icon::normalize_icon(&image.to_rgba8()))
        .map_err(|err| err.to_string())
}

impl TrayState {
    pub fn new() -> Result<Self, String> {
        let image = tray_image()?;
        let (width, height) = image.dimensions();
        let icon = tray_icon::Icon::from_rgba(image.into_raw(), width, height)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_tray_artwork_fills_canvas_and_preserves_transparency() {
        let image = tray_image().expect("decode bundled tray image");
        let (width, height) = image.dimensions();
        assert_eq!(image.as_raw().len(), (width * height * 4) as usize);
        tray_icon::Icon::from_rgba(image.clone().into_raw(), width, height)
            .expect("tray image has valid RGBA dimensions");

        let visible: Vec<_> = image
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel.0[3] > 8)
            .map(|(x, y, _)| (x, y))
            .collect();
        let min_x = visible.iter().map(|(x, _)| *x).min().unwrap();
        let max_x = visible.iter().map(|(x, _)| *x).max().unwrap();
        let min_y = visible.iter().map(|(_, y)| *y).min().unwrap();
        let max_y = visible.iter().map(|(_, y)| *y).max().unwrap();
        let longest = (max_x - min_x + 1).max(max_y - min_y + 1);
        let ratio = longest as f32 / width.max(height) as f32;
        assert!(
            (0.85..=0.95).contains(&ratio),
            "tray artwork is {longest}px on {width}x{height} canvas (ratio {ratio:.3})"
        );
        assert_eq!((width, height), (64, 64));
        assert!(min_x > 0 && min_y > 0 && max_x < width - 1 && max_y < height - 1);
        assert!(
            image.enumerate_pixels().all(|(x, y, pixel)| {
                (x != 0 && y != 0 && x != width - 1 && y != height - 1) || pixel.0[3] == 0
            }),
            "tray canvas border must remain transparent"
        );
        assert!(
            image.pixels().any(|pixel| (1..255).contains(&pixel.0[3])),
            "tray artwork must preserve its partially transparent edges"
        );
    }
}
