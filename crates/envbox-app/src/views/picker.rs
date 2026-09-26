//! Local installed-app picker overlay (search + icons + capability badges).

use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::discover::DiscoveredApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::package::{InjectionSupport, Packaging};
use crate::theme::*;
use crate::widgets::{badge, primary_btn, secondary_btn};

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let Some(state) = app.app_picker.as_ref() else {
        return Space::new(Length::Fill, Length::Fill).into();
    };

    let dim = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(iced::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.55,
                })),
                ..Default::default()
            }),
    )
    .on_press(Message::AppPickerClose);

    let title = column![
        text("选择应用")
            .size(18)
            .color(INK)
            .font(font::name_font()),
        text("从本机已安装应用中选择。商店 / AppContainer 目标可能无法注入 Runtime。")
            .size(12)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(4);

    let close_btn = button(icon(Icon::Close, MUTED, 12.0))
        .padding(Padding::from([6, 10]))
        .style(secondary_btn)
        .on_press(Message::AppPickerClose);

    let header = row![title.width(Fill), close_btn]
        .spacing(12)
        .align_y(Alignment::Center);

    let search = container(
        row![
            icon(Icon::Search, MUTED, 13.0),
            text_input("搜索应用名称或路径…", &state.query)
                .on_input(Message::AppPickerQuery)
                .padding(Padding::from([8, 10]))
                .style(input_style)
                .font(font::ui_font())
                .width(Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 12]))
    .width(Fill)
    .style(search_box_container);

    let filtered = app.filtered_picker_items();
    let count_text = if state.loading {
        "正在扫描本机应用…".to_string()
    } else {
        format!("{} 个应用", filtered.len())
    };

    let list: Element<_> = if state.loading && state.items.is_empty() {
        container(
            column![
                icon(Icon::Apps, MUTED, 22.0),
                text("正在读取开始菜单 / 桌面…")
                    .size(13)
                    .color(INK_2)
                    .font(font::name_font()),
            ]
            .spacing(8)
            .align_x(Alignment::Center),
        )
        .padding(36)
        .center_x(Fill)
        .center_y(Fill)
        .into()
    } else if filtered.is_empty() {
        container(
            column![
                icon(Icon::Search, MUTED, 22.0),
                text("未找到匹配的应用")
                    .size(13)
                    .color(INK_2)
                    .font(font::name_font()),
                text("试试其他关键字，或使用「手动添加」")
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .padding(36)
        .center_x(Fill)
        .into()
    } else {
        let col = filtered.into_iter().fold(
            column![].spacing(4),
            |c, (idx, item)| c.push(item_row(idx, item, state.selected == Some(idx))),
        );
        scrollable(col)
            .height(Fill)
            .style(dark_scrollable)
            .into()
    };

    let footer_count = text(count_text)
        .size(12)
        .color(MUTED)
        .font(font::ui_font());

    let manual_btn = button(
        text("手动添加")
            .size(12)
            .color(INK_2)
            .font(font::name_font()),
    )
    .padding(Padding::from([8, 14]))
    .style(secondary_btn)
    .on_press(Message::AppPickerManual);

    let use_btn = button(
        row![
            icon(Icon::Play, INK, 11.0),
            text("使用所选").size(12).color(INK).font(font::name_font()),
        ]
        .spacing(5)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 16]))
    .style(primary_btn)
    .on_press(Message::AppPickerUseSelected);

    let footer = row![
        footer_count.width(Fill),
        manual_btn,
        use_btn,
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let panel = container(
        column![header, search, list, footer]
            .spacing(12)
            .padding(Padding::from([18, 20])),
    )
    .width(Length::Fixed(720.0))
    .height(Length::Fixed(560.0))
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

    container(
        iced::widget::stack![
            dim,
            container(panel)
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill),
        ]
        .width(Fill)
        .height(Fill),
    )
    .width(Fill)
    .height(Fill)
    .into()
}

fn item_row<'a>(idx: usize, item: &'a DiscoveredApp, selected: bool) -> Element<'a, Message> {
    let tile = app_tile(item);

    let name = text(item.name.clone())
        .size(13)
        .color(INK)
        .font(font::name_font());

    let path_label = if item.capability.packaging == Packaging::Win32 {
        ellipsize(&item.path, 56)
    } else {
        "Windows 应用".to_string()
    };
    let path = text(path_label)
        .size(11)
        .color(MUTED)
        .font(font::ui_font());

    let source_badge = badge(
        source_label(item.source),
        ACCENT_BG,
        ACCENT_TEXT,
    );

    let support_badge = match item.capability.injection {
        InjectionSupport::Supported => None,
        InjectionSupport::Delayed => {
            Some(badge(item.capability.badge(), ACCENT_BG, ACCENT_TEXT))
        }
        InjectionSupport::Unsupported => {
            Some(badge(item.capability.badge(), DANGER_BG, DANGER_TEXT))
        }
    };

    let mut title_row = row![name, source_badge]
        .spacing(6)
        .align_y(Alignment::Center);
    if let Some(support_badge) = support_badge {
        title_row = title_row.push(support_badge);
    }
    let meta = column![
        title_row,
        path,
    ]
    .spacing(4);

    button(
        row![tile, meta]
            .spacing(12)
            .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 12]))
    .width(Fill)
    .style(move |_t, _s| button::Style {
        background: Some(iced::Background::Color(if selected {
            ACCENT_SOFT
        } else {
            CARD_IDLE
        })),
        border: iced::Border {
            color: if selected { ACCENT_LINE } else { BORDER },
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: INK,
        ..Default::default()
    })
    .on_press(Message::AppPickerSelect(idx))
    .into()
}

fn app_tile(item: &DiscoveredApp) -> Element<'static, Message> {
    if let Some(png) = &item.icon_png {
        return container(
            image(image::Handle::from_path(png.clone()))
                .width(Length::Fixed(32.0))
                .height(Length::Fixed(32.0)),
        )
        .width(Length::Fixed(36.0))
        .height(Length::Fixed(36.0))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(CARD_INNER)),
            border: iced::Border {
                color: BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
        .center_x(36.0)
        .center_y(36.0)
        .into();
    }
    crate::widgets::app_icon_badge(&item.name, 36.0)
}

fn source_label(source: &'static str) -> &'static str {
    match source {
        "start-menu" => "开始菜单",
        "desktop" => "桌面",
        "apps-folder" => "商店",
        other => other,
    }
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}
