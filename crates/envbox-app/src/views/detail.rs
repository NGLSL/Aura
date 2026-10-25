//! Right detail panel orchestration and shared layout helpers.
//!
//! The application and Profile panels live in separate modules so their editor
//! controls do not grow into one coupled view. This file keeps the public
//! `detail::view` contract used by the shell and the small shared layout pieces.

mod app;
mod profile;

use iced::widget::{column, container, scrollable, text};
use iced::{Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::message::{Message, Nav};
use crate::theme::*;

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    match app.nav {
        Nav::Apps => app::view(app),
        Nav::Profiles => profile::view(app),
        _ => container(column![text("详情").size(TEXT_SECTION).color(MUTED)])
            .padding(18)
            .width(Length::Fixed(360.0))
            .height(Fill)
            .style(|_| panel_style(PANEL_RIGHT))
            .into(),
    }
}

pub(super) fn detail_shell<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    detail_layout(content.into(), None)
}

pub(super) fn detail_editor_shell<'a>(
    content: impl Into<Element<'a, Message>>,
    actions: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    detail_layout(content.into(), Some(actions.into()))
}

fn detail_layout<'a>(
    content: Element<'a, Message>,
    actions: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let body = scrollable(container(content).width(Fill).padding(Padding {
        right: 8.0,
        ..Padding::ZERO
    }))
    .height(Fill)
    .style(dark_scrollable);
    let mut layout = column![body].spacing(12).height(Fill);
    if let Some(actions) = actions {
        layout = layout.push(container(actions).width(Fill).padding(Padding {
            right: 8.0,
            ..Padding::ZERO
        }));
    }
    container(layout)
        .padding(Padding::from([16, 12]))
        .width(Fill)
        .height(Fill)
        .style(|_| panel_style(PANEL_RIGHT))
        .into()
}

pub(super) fn section_title<'a>(title: &'a str, subtitle: &'a str) -> Element<'a, Message> {
    column![
        text(title)
            .size(TEXT_SECTION)
            .color(INK_2)
            .font(font::name_font()),
        text(subtitle)
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(2)
    .into()
}
