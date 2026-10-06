//! Applications center: search/header and application cards.

use iced::widget::{button, column, container, row, scrollable, text, text_input, tooltip, Column};
use iced::{Alignment, Border, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::*;
use crate::widgets::{app_icon_badge, primary_btn, status_dot};

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

pub fn view_center(app: &EnvBoxApp) -> Element<'_, Message> {
    let title = column![
        text("应用").size(24).color(INK).font(font::name_font()),
        text("为每次启动选择环境配置，应用仍在本机正常运行。")
            .size(12)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(2);

    let add_btn = button(
        row![
            text("+").size(15).color(INK).font(font::name_font()),
            text("添加应用").size(13).color(INK).font(font::ui_font()),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([7, 14]))
    .style(primary_btn)
    .on_press(Message::AppNew);

    let search_bar = container(
        row![
            icon(Icon::Search, MUTED, 13.0),
            text_input("搜索应用...", &app.search)
                .on_input(Message::Search)
                .padding(Padding::from([3, 5]))
                .style(borderless_input)
                .font(font::ui_font()),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([4, 10]))
    .width(Fill)
    .style(search_box_container);

    let header = column![
        title.width(Fill),
        row![add_btn, search_bar]
            .spacing(12)
            .align_y(Alignment::Center)
    ]
    .spacing(10);

    let filtered = app.filtered_apps();
    let cards: Element<_> = if filtered.is_empty() {
        if app.applications.is_empty() {
            let add_first = button(
                row![
                    icon(Icon::Play, INK, 11.0),
                    text("添加第一个应用")
                        .size(13)
                        .color(INK)
                        .font(font::name_font()),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([9, 20]))
            .style(primary_btn)
            .on_press(Message::AppNew);

            crate::widgets::empty_state(
                Icon::Apps,
                "暂无配置的应用",
                "添加本机程序或命令，并选择启动时使用的环境配置。",
                Some(add_first.into()),
            )
        } else {
            container(
                column![
                    icon(Icon::Search, MUTED, 24.0),
                    text("未找到匹配的应用")
                        .size(13)
                        .color(INK_2)
                        .font(font::name_font()),
                    text("请检查搜索关键字或尝试其他应用名称")
                        .size(12)
                        .color(MUTED)
                        .font(font::ui_font()),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .padding(40)
            .center_x(Fill)
            .into()
        }
    } else {
        let col = filtered.into_iter().fold(
            Column::new().spacing(8).padding(Padding::from([2, 0])),
            |c, a| c.push(app_card(app, a)),
        );
        scrollable(col)
            .height(Length::Fill)
            .style(dark_scrollable)
            .into()
    };

    column![header, cards]
        .spacing(10)
        .width(Fill)
        .height(Fill)
        .into()
}

fn app_card<'a>(app: &'a EnvBoxApp, a: &'a envbox_core::Application) -> Element<'a, Message> {
    let selected = app.app_draft.id == Some(a.id);
    let running_count = app.running_count(a.id);
    let id = a.id;
    let capability = app.app_capabilities.get(&id).copied();

    let status_indicator = row![
        status_dot(if running_count > 0 { SUCCESS } else { FAINT }),
        text(if running_count > 0 {
            format!("运行中 {running_count} 个")
        } else {
            "未运行".to_string()
        })
        .size(12)
        .color(if running_count > 0 { SUCCESS } else { MUTED })
        .font(font::ui_font()),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let tile = app
        .app_icons
        .get(&a.id)
        .map(|png| {
            container(
                iced::widget::image(iced::widget::image::Handle::from_path(png.clone()))
                    .width(Length::Fixed(38.0))
                    .height(Length::Fixed(38.0))
                    .content_fit(iced::ContentFit::Contain)
                    .filter_method(iced::widget::image::FilterMethod::Linear),
            )
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .style(|_| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.05, 0.07, 0.10,
                ))),
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..Default::default()
            })
            .center_x(40.0)
            .center_y(40.0)
            .into()
        })
        .unwrap_or_else(|| app_icon_badge(&a.name, 40.0));

    let mut heading = row![text(&a.name)
        .size(15)
        .color(INK)
        .font(font::name_font())
        .width(Fill)]
    .spacing(8)
    .align_y(Alignment::Center);
    let warning = if crate::package::chromium_renderer_sandbox_limited(a) {
        Some("浏览器网页沙箱可能读取宿主信息；部分环境设置受限。")
    } else {
        match capability.map(|value| value.injection) {
            Some(crate::package::InjectionSupport::Unsupported) => Some("此应用无法使用环境配置。"),
            Some(crate::package::InjectionSupport::Delayed) => Some("部分环境设置可能不生效。"),
            _ => None,
        }
    };
    if let Some(warning) = warning {
        heading = heading.push(
            tooltip(
                icon(Icon::Info, MUTED, 12.0),
                container(text(warning).size(12).font(font::ui_font()))
                    .width(260)
                    .padding(10),
                tooltip::Position::Right,
            )
            .style(inner_card_style),
        );
    }
    let profile = ellipsize(&app.profile_name(a.default_profile_id), 32);
    let metadata = row![
        text(profile)
            .size(12)
            .color(MUTED)
            .font(font::ui_font())
            .width(Fill),
        status_indicator
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let information = column![heading, metadata].spacing(6).width(Fill);
    let content = row![tile, information]
        .spacing(14)
        .align_y(Alignment::Center)
        .width(Fill);
    button(content)
        .padding(Padding::from([12, 14]))
        .width(Fill)
        .style(move |_, _| list_card_style(selected))
        .on_press(Message::AppSelect(id))
        .into()
}
