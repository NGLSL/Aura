//! Inline SVG icons (Material-style, `fill="currentColor"`).
//! Tinted via svg style — no emoji, no tofu risk.

use iced::widget::svg;
use iced::{Color, Length};

use crate::message::Nav;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Apps,
    Profiles,
    Instances,
    Audit,
    Settings,
    Minimize,
    Maximize,
    Close,
    Folder,
    Terminal,
    Search,
    Play,
    Stop,
    Globe,
    ChevronDown,
    ArrowsHorizontal,
    Info,
    ExternalLink,
    MoreHorizontal,
    Document,
    Monitor,
    Sliders,
    Claude,
    OpenAi,
    Mimo,
    Flask,
    Check,
    Trash,
}

impl Icon {
    fn svg_content(self) -> &'static str {
        match self {
            Icon::Apps => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M3 5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5zm10 0a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2V5zM3 15a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4zm10 0a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2v-4z"/></svg>"#
            }
            Icon::Profiles => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z"/></svg>"#
            }
            Icon::Instances => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 14H4V8h16v10z"/><path fill="currentColor" d="M7 10l3 2.5L7 15l1.2 1 4.2-3.5L8.2 9zM12 15h5v1.5h-5z"/></svg>"#
            }
            Icon::Audit => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M4 6.5a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm0 7a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm0 7a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm4-13h13v2H8V5.5zm0 7h13v2H8v-2zm0 7h13v2H8v-2z"/></svg>"#
            }
            Icon::Settings => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/></svg>"#
            }
            Icon::Minimize => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M5 12h14v2H5z"/></svg>"#
            }
            Icon::Maximize => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M4 4h16v16H4V4zm2 2v12h12V6H6z"/></svg>"#
            }
            Icon::Close => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>"#
            }
            Icon::Search => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M15.5 14h-.79l-.28-.27A6.471 6.471 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/></svg>"#
            }
            Icon::Play => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M7 4.5v15l13-7.5z"/></svg>"#
            }
            Icon::Stop => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor"/></svg>"#
            }
            Icon::Folder => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/></svg>"#
            }
            Icon::Terminal => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 14H4V8h16v10zm-2-1h-6v-2h6v2zM7.5 17l-1.41-1.41L8.67 13l-2.59-2.59L7.5 9l4 4-4 4z"/></svg>"#
            }
            Icon::ArrowsHorizontal => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M6.99 11L3 15l3.99 4v-3H14v-2H6.99v-3zM21 9l-3.99-4v3H10v2h7.01v3L21 9z"/></svg>"#
            }
            Icon::Globe => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2V3.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z"/></svg>"#
            }
            Icon::ChevronDown => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6 1.41-1.41z"/></svg>"#
            }
            Icon::Info => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z"/></svg>"#
            }
            Icon::ExternalLink => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/></svg>"#
            }
            Icon::MoreHorizontal => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="5" cy="12" r="2.2" fill="currentColor"/><circle cx="12" cy="12" r="2.2" fill="currentColor"/><circle cx="19" cy="12" r="2.2" fill="currentColor"/></svg>"#
            }
            Icon::Document => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z"/></svg>"#
            }
            Icon::Monitor => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M20 18c1.1 0 1.99-.9 1.99-2L22 6c0-1.1-.9-2-2-2H4c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2H0v2h24v-2h-4zM4 6h16v10H4V6z"/></svg>"#
            }
            Icon::Sliders => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M4 7h6v2H4V7zm10 0h6v2h-6V7zm-2-2.5h2v7h-2v-7zm-8 10h10v2H4v-2zm14 0h2v2h-2v-2zm-3-2.5h2v7h-2v-7z"/></svg>"#
            }
            Icon::Claude => {
                // Anthropic Claude Terracotta Starburst (8 rounded capsule rays + center circle)
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="currentColor"><rect x="10.75" y="1.5" width="2.5" height="5" rx="1.25"/><rect x="10.75" y="17.5" width="2.5" height="5" rx="1.25"/><rect x="1.5" y="10.75" width="5" height="2.5" rx="1.25"/><rect x="17.5" y="10.75" width="5" height="2.5" rx="1.25"/><rect x="4.2" y="4.2" width="2.5" height="4.5" rx="1.25" transform="rotate(-45 5.45 6.45)"/><rect x="15.8" y="15.8" width="2.5" height="4.5" rx="1.25" transform="rotate(-45 17.05 18.05)"/><rect x="16.5" y="4.2" width="2.5" height="4.5" rx="1.25" transform="rotate(45 17.75 6.45)"/><rect x="4.9" y="15.8" width="2.5" height="4.5" rx="1.25" transform="rotate(45 6.15 18.05)"/><circle cx="12" cy="12" r="2.8"/></g></svg>"#
            }
            Icon::OpenAi => {
                // Official Simple Icons OpenAI vector path
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M22.28 9.82a5.98 5.98 0 0 0-.51-4.91 6.05 6.05 0 0 0-6.51-2.9A6.07 6.07 0 0 0 4.98 4.18a5.98 5.98 0 0 0-4 2.9 6.05 6.05 0 0 0 .74 7.1 5.98 5.98 0 0 0 .51 4.91 6.05 6.05 0 0 0 6.51 2.9 5.98 5.98 0 0 0 4.52 1.91 6.06 6.06 0 0 0 5.77-4.2 5.99 5.99 0 0 0 4-2.9 6.06 6.06 0 0 0-.75-7.08zm-9.02 12.61a4.48 4.48 0 0 1-2.88-1.04l.14-.08 4.78-2.76c.24-.14.39-.4.39-.68v-6.74l2.02 1.17c.02.01.04.03.04.05v5.58a4.5 4.5 0 0 1-4.49 4.5zm-9.66-4.13a4.47 4.47 0 0 1-.54-3.01l.15.08 4.78 2.76c.24.14.54.14.78 0l5.84-3.37v2.33c0 .02-.01.05-.03.06L9.74 19.95a4.5 4.5 0 0 1-6.14-1.65zM2.34 7.9a4.48 4.48 0 0 1 2.37-1.98v5.68c0 .28.15.54.39.68l5.81 3.35-2.02 1.17a.08.08 0 0 1-.07 0l-4.83-2.79A4.5 4.5 0 0 1 2.34 7.9zm16.6 3.85L13.1 8.36 15.12 7.2a.08.08 0 0 1 .07 0l4.83 2.79a4.49 4.49 0 0 1-.68 8.1v-5.67a.79.79 0 0 0-.4-.67zm2.01-3.02l-.14-.09-4.77-2.78a.78.78 0 0 0-.79 0L9.41 9.23V6.9a.07.07 0 0 1 .03-.06l4.83-2.79a4.5 4.5 0 0 1 6.68 4.66zM8.31 12.86l-2.02-1.16a.08.08 0 0 1-.04-.06V6.07a4.5 4.5 0 0 1 7.38-3.45l-.15.08-4.78 2.76a.79.79 0 0 0-.39.68zm1.1-2.36l2.6-1.5 2.6 1.5v3l-2.6 1.5-2.6-1.5z"/></svg>"#
            }
            Icon::Mimo => {
                // Mimo bold clean M
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M4 4h4.2l3.8 6.8L15.8 4H20v16h-3.2v-9.8L12.5 17h-1L7.2 10.2V20H4V4z"/></svg>"#
            }
            Icon::Flask => {
                // Probe conical science flask with liquid level
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M19 19a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2c0-.5.2-1 .5-1.4L9 11V5H8V3h8v2h-1v6l3.5 6.6c.3.4.5.9.5 1.4zm-9-7L7.2 17h9.6L14 12V5h-4v7z"/></svg>"#
            }
            Icon::Check => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/></svg>"#
            }
            Icon::Trash => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>"#
            }
        }
    }
}

pub fn svg_handle(xml: &str) -> svg::Handle {
    svg::Handle::from_memory(xml.as_bytes().to_vec())
}

/// Fixed-size tinted icon.
pub fn icon<'a>(name: Icon, color: Color, size: f32) -> svg::Svg<'a, iced::Theme> {
    svg(svg_handle(name.svg_content()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_t, _s| svg::Style { color: Some(color) })
}

pub fn nav_icon_for(nav: Nav) -> Icon {
    match nav {
        Nav::Apps => Icon::Apps,
        Nav::Workspaces => Icon::Profiles,
        Nav::Profiles => Icon::Profiles,
        Nav::Instances => Icon::Instances,
        Nav::Audit => Icon::Audit,
        Nav::Settings => Icon::Settings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::{layout, mouse, renderer, widget::Tree, Layout};
    use iced::{Element, Font, Pixels, Point, Rectangle, Size, Theme};

    #[test]
    #[ignore = "writes UI snapshots; requires isolated ENVBOX_CONFIG_ROOT and AURA_RENDER_SNAPSHOT_DIR"]
    fn render_display_scale_snapshots() {
        assert!(std::env::var_os("ENVBOX_CONFIG_ROOT").is_some());
        let output = std::path::PathBuf::from(
            std::env::var_os("AURA_RENDER_SNAPSHOT_DIR").expect("snapshot output directory"),
        );
        std::fs::create_dir_all(&output).unwrap();
        let ui_font = crate::font::install();
        let (mut app, _) = crate::app::EnvBoxApp::new();
        app.audit_events.clear();
        let mut renderer = iced::Renderer::new(ui_font, Pixels(16.0));
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for nav in [Nav::Apps, Nav::Audit, Nav::Settings] {
                use iced::advanced::Renderer as _;
                app.nav = nav;
                renderer.clear();
                let viewport =
                    iced_graphics::Viewport::with_physical_size(Size::new(2560, 1440), scale);
                let bounds = Rectangle::with_size(viewport.logical_size());
                let element = app.view();
                let mut tree = Tree::new(&element);
                let node = element.as_widget().layout(
                    &mut tree,
                    &renderer,
                    &layout::Limits::new(viewport.logical_size(), viewport.logical_size()),
                );
                element.as_widget().draw(
                    &tree,
                    &mut renderer,
                    &Theme::Dark,
                    &renderer::Style::default(),
                    Layout::new(&node),
                    mouse::Cursor::Unavailable,
                    &bounds,
                );
                let mut pixels = tiny_skia::Pixmap::new(2560, 1440).unwrap();
                let mut mask = tiny_skia::Mask::new(2560, 1440).unwrap();
                renderer.draw(
                    &mut pixels.as_mut(),
                    &mut mask,
                    &viewport,
                    &[bounds],
                    crate::theme::WINDOW,
                    &[] as &[&str],
                );
                // The CPU renderer presents BGR to softbuffer; PNG expects RGB.
                for pixel in pixels.data_mut().chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                pixels
                    .save_png(output.join(format!("{nav:?}-{}.png", (scale * 100.0) as u32)))
                    .unwrap();
            }
        }
    }

    #[test]
    fn icons_stay_inside_widget_bounds_across_display_scales() {
        // Exercise Aura's SVG widget, including its tint and fixed-size layout,
        // through the production CPU renderer. Reuse it while DPI changes so
        // the test also reaches the vector raster cache.
        let mut renderer = iced::Renderer::new(Font::DEFAULT, Pixels(14.0));
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 1.0] {
            for (name, size) in [
                (Icon::Apps, 18.0),
                (Icon::Profiles, 18.0),
                (Icon::Instances, 18.0),
                (Icon::Audit, 18.0),
                (Icon::Settings, 18.0),
                (Icon::Document, 28.0),
                (Icon::Play, 11.0),
            ] {
                use iced::advanced::Renderer as _;
                renderer.clear();
                let element: Element<'_, ()> = icon(name, Color::WHITE, size).into();
                let mut tree = Tree::new(&element);
                let node = element
                    .as_widget()
                    .layout(
                        &mut tree,
                        &renderer,
                        &layout::Limits::new(Size::ZERO, Size::new(320.0, 240.0)),
                    )
                    .move_to(Point::new(80.0, 100.0));
                let bounds = node.bounds();
                let viewport_bounds = Rectangle::with_size(Size::new(320.0, 240.0));
                element.as_widget().draw(
                    &tree,
                    &mut renderer,
                    &Theme::Dark,
                    &renderer::Style::default(),
                    Layout::new(&node),
                    mouse::Cursor::Unavailable,
                    &viewport_bounds,
                );
                let width = (320.0 * scale) as u32;
                let height = (240.0 * scale) as u32;
                let mut pixels = tiny_skia::Pixmap::new(width, height).unwrap();
                let mut mask = tiny_skia::Mask::new(width, height).unwrap();
                let viewport =
                    iced_graphics::Viewport::with_physical_size(Size::new(width, height), scale);
                renderer.draw(
                    &mut pixels.as_mut(),
                    &mut mask,
                    &viewport,
                    &[viewport_bounds],
                    Color::TRANSPARENT,
                    &[] as &[&str],
                );
                let expected = bounds * scale as f32;
                let mut visible = 0;
                for (i, pixel) in pixels.pixels().iter().enumerate() {
                    if pixel.alpha() == 0 {
                        continue;
                    }
                    visible += 1;
                    let x = (i as u32 % width) as f32;
                    let y = (i as u32 / width) as f32;
                    assert!(
                        x >= expected.x.floor() && x < (expected.x + expected.width).ceil()
                            && y >= expected.y.floor() && y < (expected.y + expected.height).ceil(),
                        "{name:?} at {scale}x escaped its widget: pixel ({x}, {y}), expected {expected:?}",
                    );
                }
                assert!(visible > 0, "{name:?} at {scale}x was not rendered");
            }
        }
    }
}
