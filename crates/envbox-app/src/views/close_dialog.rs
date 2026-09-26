//! Close confirmation shown by the title-bar X and native close requests.

use iced::widget::{button, checkbox, column, container, mouse_area, row, text, Space};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::message::Message;
use crate::theme::{BORDER_SOFT, INK, INK_2, MUTED, PANEL};
use crate::widgets::{danger_btn, primary_btn, secondary_btn};

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let dim = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(iced::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.62,
                })),
                ..Default::default()
            }),
    )
    .on_press(Message::WindowCloseCancel);

    let mut content = column![
        text("关闭 Aura")
            .size(20)
            .color(INK)
            .font(font::name_font()),
        text("已启动的应用可以继续运行。你想如何处理 Aura 窗口？")
            .size(12)
            .color(INK_2)
            .font(font::ui_font()),
        checkbox("记住我的选择，以后不再询问", app.remember_close_choice)
            .on_toggle(Message::WindowRememberChoice)
            .size(14)
            .text_size(12),
    ]
    .spacing(15);

    if let Some(error) = &app.close_error {
        content = content.push(
            text(error)
                .size(12)
                .color(iced::Color::from_rgb(1.0, 0.45, 0.45)),
        );
    }

    content = content.push(
        row![
            button(text("取消").size(12).color(MUTED).font(font::ui_font()))
                .on_press(Message::WindowCloseCancel)
                .padding(Padding::from([8, 14]))
                .style(secondary_btn),
            button(text("退出 Aura").size(12).color(INK).font(font::ui_font()))
                .on_press(Message::WindowExit)
                .padding(Padding::from([8, 14]))
                .style(danger_btn),
            button(
                text("最小化到托盘")
                    .size(12)
                    .color(INK)
                    .font(font::name_font())
            )
            .on_press(Message::WindowCloseToTray)
            .padding(Padding::from([8, 14]))
            .style(primary_btn),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    let panel = container(content.padding(Padding::from([22, 24])))
        .width(Length::Fixed(480.0))
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
