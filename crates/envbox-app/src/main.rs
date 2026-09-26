//! EnvBox GUI entry — assembly only.
//! Business state: `app`. Views: `views`. Styles: `theme`. Font: `font`.
//! Injection logic never lives in this crate's UI layer.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod app_icon;
mod combo_overlay;
mod close_behavior;
mod discover;
mod package;
mod font;
mod icons;
mod message;
mod options;
mod theme;
#[cfg(windows)]
mod tray;
mod version;
mod views;
mod widgets;

use iced::{Size, Theme};

fn main() -> iced::Result {
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

fn window_icon() -> Option<iced::window::Icon> {
    iced::window::icon::from_file_data(include_bytes!("../../../icons/256x256.png"), None).ok()
}
