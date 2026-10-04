//! Three-column shell with custom title bar (no native chrome).

use iced::widget::{button, column, container, mouse_area, row, text, Space};
use iced::{Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::icons::{icon, Icon};
use crate::message::{Message, StatusKind};
use crate::theme::{self, win_button, win_close_button, MUTED, WINDOW};

use super::{
    apps, audit, close_dialog, detail, instances, nav, picker, profiles, settings, unsaved_dialog,
};

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    use crate::message::Nav;

    let body = match app.nav {
        Nav::Apps | Nav::Profiles => {
            row![nav::view(app), center(app), detail::view(app)].spacing(0)
        }
        Nav::Workspaces | Nav::Instances | Nav::Audit | Nav::Settings => {
            row![nav::view(app), center_full(app)].spacing(0)
        }
    };

    let mut shell = column![title_bar()].spacing(0);
    if !app.status.is_empty() {
        shell = shell.push(status_banner(app));
    }
    let shell = shell.push(body);

    let base = container(shell)
        .width(Fill)
        .height(Fill)
        .padding(1)
        .style(theme::shell_style);

    let content: Element<'_, Message> = if app.app_picker.is_some() {
        iced::widget::stack![base, picker::view(app)]
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        base.into()
    };

    let content = if app.close_dialog {
        iced::widget::stack![content, close_dialog::view(app)]
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        content
    };

    if app.unsaved_dialog_open() {
        iced::widget::stack![content, unsaved_dialog::view(app)]
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        content
    }
}

fn status_banner(app: &EnvBoxApp) -> Element<'_, Message> {
    let (background, foreground) = match app.status_kind {
        StatusKind::Info => (theme::ACCENT_BG, theme::ACCENT_TEXT),
        StatusKind::Success => (theme::SUCCESS_BG, theme::SUCCESS_TEXT),
        StatusKind::Error => (theme::DANGER_BG, theme::DANGER_TEXT),
    };
    container(
        row![
            text(&app.status).size(12).color(foreground).width(Fill),
            button(icon(Icon::Close, foreground, 11.0))
                .on_press(Message::StatusDismiss)
                .style(win_button)
                .padding(Padding::from([3, 6])),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
    )
    .width(Fill)
    .padding(Padding::from([7, 16]))
    .style(move |_| container::Style {
        background: Some(iced::Background::Color(background)),
        ..Default::default()
    })
    .into()
}

/// Custom title bar: drag region + window controls (matches mockup chrome).
fn title_bar() -> Element<'static, Message> {
    let controls = row![
        button(icon(Icon::Minimize, MUTED, 12.0))
            .on_press(Message::WindowMinimize)
            .style(win_button)
            .padding(Padding::from([8, 12])),
        button(icon(Icon::Maximize, MUTED, 12.0))
            .on_press(Message::WindowToggleMaximize)
            .style(win_button)
            .padding(Padding::from([8, 12])),
        button(icon(Icon::Close, MUTED, 12.0))
            .on_press(Message::WindowClose)
            .style(win_close_button)
            .padding(Padding::from([8, 12])),
    ]
    .spacing(2);

    let bar = row![Space::with_width(Length::Fill), controls,]
        .align_y(iced::Alignment::Center)
        .padding(Padding::from([6, 8]));

    // Entire strip is draggable except the control buttons.
    mouse_area(
        container(bar)
            .width(Fill)
            .height(Length::Fixed(36.0))
            .style(theme::titlebar_style),
    )
    .on_press(Message::WindowDrag)
    .into()
}

fn center(app: &EnvBoxApp) -> Element<'_, Message> {
    use crate::message::Nav;
    let body: Element<_> = match app.nav {
        Nav::Apps => apps::view_center(app),
        Nav::Profiles => profiles::view_center(app),
        _ => apps::view_center(app),
    };

    container(body)
        .padding(Padding::from([14, 18]))
        .width(Fill)
        .height(Fill)
        .style(|_| theme::panel_style(WINDOW))
        .into()
}

fn center_full(app: &EnvBoxApp) -> Element<'_, Message> {
    use crate::message::Nav;
    let body: Element<_> = match app.nav {
        Nav::Instances => instances::view_full(app),
        Nav::Audit => audit::view_full(app),
        Nav::Settings => settings::view(app),
        Nav::Workspaces => super::workspaces::view(app),
        _ => apps::view_center(app),
    };

    container(body)
        .padding(Padding::from([20, 28]))
        .width(Fill)
        .height(Fill)
        .style(|_| theme::panel_style(WINDOW))
        .into()
}
