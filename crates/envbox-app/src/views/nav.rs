//! Left navigation column.

use iced::widget::{button, column, container, image, row, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, nav_icon_for};
use crate::message::{Message, Nav};
use crate::theme::{self, BORDER, FAINT, INK, INK_2, MUTED, SUCCESS};
use crate::widgets::status_dot;

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    // Product mark: rounded icon container matching Figure 2
    let mark = container(
        image(image::Handle::from_bytes(
            include_bytes!("../../../../icons/64x64.png").as_slice(),
        ))
        .width(Length::Fixed(36.0))
        .height(Length::Fixed(36.0)),
    )
    .width(Length::Fixed(40.0))
    .height(Length::Fixed(40.0))
    .style(|_| container::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgb(0.08, 0.12, 0.20))),
        border: iced::Border {
            color: BORDER,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..Default::default()
    })
    .center_x(40.0)
    .center_y(40.0);

    let logo = row![
        mark,
        column![
            text("Aura")
                .size(19)
                .color(INK)
                .font(font::name_font()),
            text("EnvBox")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        ]
        .spacing(2),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let items = Nav::ALL.into_iter().fold(column![].spacing(6), |col, n| {
        let selected = app.nav == n;
        let icon_color = if selected { INK } else { MUTED };
        let ic = icon(nav_icon_for(n), icon_color, 18.0);
        let label = text(n.label())
            .size(14)
            .color(if selected { INK } else { INK_2 })
            .font(font::ui_font());
        let btn = button(row![ic, label].spacing(12).align_y(Alignment::Center))
            .width(Fill)
            .padding(Padding::from([10, 14]))
            .style(move |_t, _s| theme::nav_btn_style(selected))
            .on_press(Message::Nav(n));
        col.push(btn)
    });

    let footer = column![
        row![
            status_dot(SUCCESS),
            text("Aura 1.0.0")
                .size(12)
                .color(INK_2)
                .font(font::ui_font()),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        text("EnvBox Core · 2025.03")
            .size(11)
            .color(FAINT)
            .font(font::ui_font()),
    ]
    .spacing(4);

    container(
        column![
            logo,
            container(text(" ")).height(Length::Fixed(12.0)),
            items,
            container(text(" ")).height(Fill),
            footer,
        ]
        .spacing(8)
        .padding(18)
        .width(Fill)
        .height(Fill),
    )
    .width(Length::Fixed(220.0))
    .height(Fill)
    .style(theme::sidebar_style)
    .into()
}
