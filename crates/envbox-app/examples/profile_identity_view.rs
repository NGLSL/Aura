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
    let run_view = std::env::var_os("ENVBOX_UI_FIXTURE_PROFILE_RUN").is_some();
    let width = std::env::var("ENVBOX_UI_FIXTURE_WIDTH")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| *value >= 800.0 && *value <= 1800.0)
        .unwrap_or(1400.0);
    let ui_font = font::install();
    iced::application(
        "Aura Profile acceptance fixture",
        app::EnvBoxApp::update,
        app::EnvBoxApp::view,
    )
    .theme(|_| iced::Theme::Dark)
    .default_font(ui_font)
    .window(iced::window::Settings {
        size: iced::Size::new(width, 880.0),
        decorations: false,
        ..Default::default()
    })
    .run_with(move || {
        let (mut app, _) = app::EnvBoxApp::new();
        if std::env::var_os("ENVBOX_UI_FIXTURE_APPS").is_some() {
            app.applications = ["Google Chrome", "ChatGPT"]
                .into_iter()
                .map(|name| envbox_core::Application {
                    id: uuid::Uuid::new_v4(),
                    name: name.into(),
                    launch: envbox_core::LaunchTarget::Executable {
                        path: std::path::PathBuf::from("C:\\Aura-UI-Fixture\\application.exe"),
                    },
                    working_directory: None,
                    arguments: Vec::new(),
                    default_profile_id: profile_id,
                    inherit_children: true,
                    console_host: envbox_core::ConsoleHost::Direct,
                    audit: false,
                })
                .collect();
            let id = app.applications[0].id;
            app.select_app(id);
            return (app, iced::Task::none());
        }
        let _ = app.update(message::Message::Nav(message::Nav::Profiles));
        let _ = app.update(message::Message::ProfileSelect(profile_id));
        if !run_view {
            let _ = app.update(message::Message::ProfileEdit);
            let _ = app.update(message::Message::ProfileAdvancedToggle);
        }
        if std::env::var_os("ENVBOX_UI_FIXTURE_DOH").is_some() {
            app.profile_draft.dns_editor.draft.transport = app::dns_editor::TransportChoice::Doh;
        }
        if std::env::var_os("ENVBOX_UI_FIXTURE_LONG_INPUTS").is_some() {
            app.profile_draft.dns_editor.draft.url = "https://cloudflare-dns.com/dns-query".into();
            app.profile_panes = iced::widget::pane_grid::State::with_configuration(
                iced::widget::pane_grid::Configuration::Split {
                    axis: iced::widget::pane_grid::Axis::Vertical,
                    ratio: 0.65,
                    a: Box::new(iced::widget::pane_grid::Configuration::Pane(false)),
                    b: Box::new(iced::widget::pane_grid::Configuration::Pane(true)),
                },
            );
        }
        if std::env::var_os("ENVBOX_UI_FIXTURE_SAVE_DOH").is_some() {
            app.profile_draft.dns_mode = message::DnsChoice::VirtualView;
            app.profile_draft.dns_editor.draft.transport = app::dns_editor::TransportChoice::Doh;
            app.profile_draft.dns_editor.draft.url = "https://resolver.example/dns-query".into();
            let _ = app.update(message::Message::ProfileSave);
            let saved = app.store.load_profiles().expect("saved fixture Profile");
            assert_eq!(saved.profiles[0].dns.upstreams.len(), 1);
            assert!(matches!(&saved.profiles[0].dns.upstreams[0],
                envbox_core::DnsUpstream::Doh { bootstrap_ips, .. } if bootstrap_ips.is_empty()));
            assert!(app.profile_draft.dns_editor.draft.url.is_empty());
            let _ = app.update(message::Message::ProfileEdit);
            let _ = app.update(message::Message::ProfileSave);
            assert_eq!(
                app.store.load_profiles().unwrap().profiles[0]
                    .dns
                    .upstreams
                    .len(),
                1
            );
        }
        // The fixture deliberately does not execute Supervisor requests.
        app.workspace_management.busy = false;
        (app, iced::Task::none())
    })
}
