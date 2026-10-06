//! Opt-in desktop acceptance fixture using the real Profile editor and renderer.
//! Run with a dedicated ENVBOX_CONFIG_ROOT containing a Profile. Does not claim
//! the product singleton or install a tray icon; no instance is launched here.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![allow(dead_code)]

#[path = "../src/app.rs"]
mod app;
#[path = "../src/app_icon.rs"]
mod app_icon;
#[path = "../src/close_behavior.rs"]
mod close_behavior;
#[path = "../src/combo_overlay.rs"]
mod combo_overlay;
#[path = "../src/discover.rs"]
mod discover;
#[path = "../src/discover_commands.rs"]
mod discover_commands;
#[path = "../src/font.rs"]
mod font;
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/message.rs"]
mod message;
#[path = "../src/options.rs"]
mod options;
#[path = "../src/package.rs"]
mod package;
#[cfg(windows)]
#[path = "../src/singleton.rs"]
mod singleton;
#[path = "../src/theme.rs"]
mod theme;
#[cfg(windows)]
#[path = "../src/tray.rs"]
mod tray;
#[path = "../src/updater.rs"]
pub(crate) mod updater;
#[path = "../src/version.rs"]
mod version;
#[path = "../src/views/mod.rs"]
mod views;
#[path = "../src/widgets.rs"]
mod widgets;

fn main() -> iced::Result {
    let root = std::env::var_os("ENVBOX_CONFIG_ROOT").expect("dedicated fixture root required");
    let store = envbox_storage::ConfigStore::new(root);
    let document = store.load_profiles().expect("fixture Profile document");
    assert_eq!(
        document.profiles.len(),
        1,
        "exactly one fixture Profile required"
    );
    assert!(
        !document.profiles[0].identity.is_host(),
        "configured identity required"
    );
    assert!(
        store
            .load_applications()
            .expect("fixture applications")
            .applications
            .is_empty(),
        "desktop fixture must not expose application launch actions"
    );
    let profile_id = document.profiles[0].id;
    let ui_font = font::install();
    iced::application(
        "Aura Profile acceptance fixture",
        app::EnvBoxApp::update,
        app::EnvBoxApp::view,
    )
    .theme(|_| iced::Theme::Dark)
    .default_font(ui_font)
    .window(iced::window::Settings {
        size: iced::Size::new(1400.0, 880.0),
        decorations: false,
        ..Default::default()
    })
    .run_with(move || {
        let (mut app, _) = app::EnvBoxApp::new();
        let _ = app.update(message::Message::Nav(message::Nav::Profiles));
        let _ = app.update(message::Message::ProfileSelect(profile_id));
        let _ = app.update(message::Message::ProfileEdit);
        let _ = app.update(message::Message::ProfileAdvancedToggle);
        (app, iced::Task::none())
    })
}
