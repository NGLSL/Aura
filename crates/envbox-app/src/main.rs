//! EnvBox GUI entry — assembly only.
//! Business state: `app`. Views: `views`. Styles: `theme`. Font: `font`.
//! Injection logic never lives in this crate's UI layer.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod app_icon;
mod close_behavior;
mod combo_overlay;
mod discover;
mod discover_commands;
mod font;
mod icons;
mod message;
mod options;
mod package;
#[cfg(windows)]
mod singleton;
mod theme;
#[cfg(windows)]
mod tray;
mod version;
mod views;
mod widgets;

use iced::{Size, Theme};

fn main() -> iced::Result {
    #[cfg(windows)]
    match singleton::claim() {
        Ok(singleton::Claim::Primary) => {}
        Ok(singleton::Claim::AlreadyRunning) => return Ok(()),
        Err(error) => {
            show_startup_error(&error);
            std::process::exit(1);
        }
    }

    let ui_font = font::install();
    iced::application("Aura", app::EnvBoxApp::update, app::EnvBoxApp::view)
        .subscription(app::EnvBoxApp::subscription)
        .exit_on_close_request(false)
        .theme(|_| Theme::Dark)
        .default_font(ui_font)
        .window(iced::window::Settings {
            size: Size::new(1400.0, 880.0),
            min_size: Some(Size::new(1100.0, 720.0)),
            position: iced::window::Position::Centered,
            // Custom title bar matches the dark mockup (no native chrome).
            decorations: false,
            icon: window_icon(),
            ..Default::default()
        })
        .run_with(app::EnvBoxApp::new)
}

#[cfg(windows)]
fn show_startup_error(error: &str) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let text: Vec<u16> = format!("Aura 无法启动：{error}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("Aura"),
            MB_ICONERROR | MB_OK,
        );
    }
}

fn window_icon() -> Option<iced::window::Icon> {
    iced::window::icon::from_file_data(include_bytes!("../../../icons/256x256.png"), None).ok()
}
