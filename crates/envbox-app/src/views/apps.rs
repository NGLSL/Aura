//! Applications center: search/header and application cards.

use iced::widget::{button, column, container, pick_list, row, scrollable, text, text_input, Column};
use iced::{Alignment, Border, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::*;
use crate::widgets::{app_icon_badge, badge, primary_btn, secondary_btn, status_dot};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunChoice {
    label: String,
    profile_id: Option<uuid::Uuid>,
}

impl std::fmt::Display for RunChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

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
    .width(Length::Fixed(210.0))
    .style(search_box_container);

    let header = row![title.width(Fill), add_btn, search_bar]
        .spacing(12)
        .align_y(Alignment::Center);

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
    use envbox_core::LaunchTarget;

    let selected = app.app_draft.id == Some(a.id);
    let running_count = app.running_count(a.id);
    let id = a.id;
    let capability = app.app_capabilities.get(&id).copied();
    let unsupported = capability
        .is_some_and(|cap| cap.injection == crate::package::InjectionSupport::Unsupported);

    let path_label = match &a.launch {
        LaunchTarget::Command { command } => command.clone(),
        LaunchTarget::Executable { path } => path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
        LaunchTarget::Packaged { .. } => "Windows 应用".to_string(),
    };

    let profile_badge = badge(
        &app.profile_name(a.default_profile_id),
        ACCENT_BG,
        ACCENT_TEXT,
    );

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

    let run_button = button(
        row![
            icon(Icon::Play, INK, 11.0),
            text("运行").size(12).color(INK).font(font::name_font())
        ]
        .spacing(5)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([5, 14]))
    .style(if unsupported {
        secondary_btn
    } else {
        primary_btn
    })
    .on_press_maybe((!unsupported).then_some(Message::AppRunId(id)));

    let mut choices: Vec<RunChoice> = if unsupported {
        Vec::new()
    } else {
        app.profiles
            .iter()
            .filter(|profile| profile.id != a.default_profile_id)
            .map(|profile| {
                let duplicate_name = app
                    .profiles
                    .iter()
                    .filter(|other| other.name == profile.name)
                    .count()
                    > 1;
                let label = if duplicate_name {
                    format!("{} ({})", profile.name, &profile.id.to_string()[..8])
                } else {
                    profile.name.clone()
                };
                RunChoice {
                    label,
                    profile_id: Some(profile.id),
                }
            })
            .collect()
    };
    choices.push(RunChoice {
        label: "在本机直接运行".to_string(),
        profile_id: None,
    });
    let run_picker = pick_list(choices, None::<RunChoice>, move |choice| {
        Message::AppRunWithId(id, choice.profile_id)
    })
    .placeholder("其他方式")
    .style(pick_style)
    .menu_style(pick_menu)
    .padding(Padding::from([5, 8]))
    .font(font::ui_font())
    .text_size(12)
    .width(Length::Fixed(190.0));

    let action_row = row![run_button, run_picker]
        .spacing(5)
        .align_y(Alignment::Center);
    let mut action_stack = column![action_row]
        .align_x(Alignment::End)
        .width(Length::Shrink);
    if running_count > 0 {
        action_stack = action_stack.push(
            button(
                row![
                    icon(Icon::Monitor, INK_2, 10.0),
                    text(format!("查看实例 ({running_count})"))
                        .size(11)
                        .color(INK_2)
                        .font(font::ui_font()),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([4, 8]))
            .style(secondary_btn)
            .on_press(Message::InstanceFilter(Some(id))),
        );
    }

    let tile = app
        .app_icons
        .get(&a.id)
        .map(|png| {
            container(
                iced::widget::image(iced::widget::image::Handle::from_path(png.clone()))
                    .width(Length::Fixed(36.0))
                    .height(Length::Fixed(36.0)),
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

    let meta_cmd = row![
        icon(Icon::ArrowsHorizontal, FAINT, 11.0),
        text(ellipsize(&path_label, 50))
            .size(11)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(5)
    .align_y(Alignment::Center);

    let mut top_left = row![
        text(&a.name).size(15).color(INK).font(font::name_font()),
        profile_badge,
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(status_badge) = capability_badge(capability) {
        top_left = top_left.push(status_badge);
    }

    let left_info = column![top_left, meta_cmd].spacing(6).width(Length::Fill);

    // Keep the action stack at its natural width. The left side owns the
    // remaining space, so the actions stay against the card's right edge.
    let right_column = column![status_indicator, action_stack]
        .spacing(6)
        .align_x(Alignment::End)
        .width(Length::Shrink);

    // Only the information side is a selection hit area. Keeping the action
    // stack outside it prevents a transparent mouse area from intercepting
    // the Run, menu, and instance-filter buttons.
    let left_content = iced::widget::mouse_area(
        row![tile, left_info]
            .spacing(14)
            .align_y(Alignment::Center)
            .width(Fill),
    )
    .on_press(Message::AppSelect(id));

    let card = container(
        row![left_content, right_column]
            .spacing(12)
            .align_y(Alignment::Center)
            .width(Fill),
    )
    .padding(Padding::from([12, 16]))
    .width(Fill)
    .style(move |_| card_style(selected));

    card.into()
}

fn capability_badge(
    capability: Option<crate::package::Capability>,
) -> Option<Element<'static, Message>> {
    use crate::package::InjectionSupport;

    match capability.map(|cap| cap.injection) {
        Some(InjectionSupport::Unsupported) => {
            Some(badge("无法使用环境配置", DANGER_BG, DANGER_TEXT))
        }
        Some(InjectionSupport::Delayed) => {
            Some(badge("部分环境可能不生效", ACCENT_BG, ACCENT_TEXT))
        }
        Some(InjectionSupport::Supported) | None => None,
    }
}
