//! Protect draft changes when leaving an application or Profile editor.

use iced::widget::{button, column, container, mouse_area, row, text, Space};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::message::Message;
use crate::theme::{BORDER_SOFT, DANGER_TEXT, INK, INK_2, MUTED, PANEL};
use crate::widgets::{primary_btn, secondary_btn};

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let dim = container(Space::new(Length::Fill, Length::Fill))
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(iced::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.66,
            })),
            ..Default::default()
        });
    let dim = mouse_area(dim).on_press(Message::UnsavedCancel);

    let mut content = column![
        text("有未保存的修改")
            .size(20)
            .color(INK)
            .font(font::name_font()),
        text("离开前要保存当前编辑的内容吗？")
            .size(12)
            .color(INK_2)
            .font(font::ui_font()),
    ]
    .spacing(14);

    if let Some(error) = &app.unsaved_error {
        content = content.push(text(error).size(12).color(DANGER_TEXT));
    }

    content = content.push(
        row![
            button(text("继续编辑").size(12).color(MUTED))
                .on_press(Message::UnsavedCancel)
                .padding(Padding::from([8, 14]))
                .style(secondary_btn),
            button(text("放弃修改").size(12).color(INK_2))
                .on_press(Message::UnsavedDiscard)
                .padding(Padding::from([8, 14]))
                .style(secondary_btn),
            button(text("保存并继续").size(12).color(INK))
                .on_press(Message::UnsavedSave)
                .padding(Padding::from([8, 14]))
                .style(primary_btn),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    let panel = container(content.padding(Padding::from([22, 24])))
        .width(Length::Fixed(440.0))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(PANEL)),
            text_color: Some(INK),
            border: iced::Border {
                color: BORDER_SOFT,
                width: 1.0,
                radius: 12.0.into(),
            },
            ..Default::default()
        });

    iced::widget::stack![
        dim,
        container(panel)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
