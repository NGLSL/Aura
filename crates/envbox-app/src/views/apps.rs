//! Applications center: search/header, app cards, bottom instances/audit panel.

use iced::widget::{
    button, column, container, row, scrollable, text, text_input, Column,
};
use iced::{Alignment, Border, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::*;
use crate::widgets::{app_icon_badge, badge, danger_btn, primary_btn, secondary_btn, status_dot};

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

fn format_uptime(started_at: std::time::SystemTime) -> String {
    let secs = match started_at.elapsed() {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    };
    if secs < 60 {
        format!("已运行 {} 秒", secs.max(1))
    } else {
        format!("已运行 {} 分钟", (secs / 60).max(1))
    }
}

pub fn view_center(app: &EnvBoxApp) -> Element<'_, Message> {
    let title = column![
        text("应用").size(24).color(INK).font(font::name_font()),
        text("在隔离的环境中运行应用程序，每个进程拥有独立的环境配置。")
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
                    text("添加第一个应用").size(13).color(INK).font(font::name_font()),
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
                "在 Aura 中添加可执行程序或命令行，并为其指定独立的环境 Profile。",
                Some(add_first.into()),
            )
        } else {
            container(
                column![
                    icon(Icon::Search, MUTED, 24.0),
                    text("未找到匹配的应用").size(13).color(INK_2).font(font::name_font()),
                    text("请检查搜索关键字或尝试其他应用名称").size(12).color(MUTED).font(font::ui_font()),
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

    let bottom = bottom_panel(app);

    column![header, cards, bottom]
        .spacing(10)
        .width(Fill)
        .height(Fill)
        .into()
}

fn app_card<'a>(app: &'a EnvBoxApp, a: &'a envbox_core::Application) -> Element<'a, Message> {
    use envbox_core::LaunchTarget;

    let selected = app.app_draft.id == Some(a.id);
    let running = app.is_app_running(a.id);
    let id = a.id;

    let path_label = match &a.launch {
        LaunchTarget::Command { command } => command.clone(),
        LaunchTarget::Executable { path } => path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
    };

    let profile_badge = badge(
        &app.profile_name(a.default_profile_id),
        ACCENT_BG,
        ACCENT_TEXT,
    );

    let status_indicator = row![
        status_dot(if running { SUCCESS } else { FAINT }),
        text(if running { "运行中" } else { "已停止" })
            .size(12)
            .color(if running { SUCCESS } else { MUTED })
            .font(font::ui_font()),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let action_btn = if running {
        button(
            row![
                icon(Icon::Stop, DANGER_TEXT, 11.0),
                text("停止").size(12).color(DANGER_TEXT).font(font::name_font())
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([5, 14]))
        .style(danger_btn)
        .on_press(Message::AppStopId(id))
    } else {
        button(
            row![
                icon(Icon::Play, INK, 11.0),
                text("运行").size(12).color(INK).font(font::name_font())
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([5, 14]))
        .style(primary_btn)
        .on_press(Message::AppRunId(id))
    };

    let tile = app.app_icons.get(&a.id).map(|png| {
        container(
            iced::widget::image(iced::widget::image::Handle::from_path(png.clone()))
                .width(Length::Fixed(36.0))
                .height(Length::Fixed(36.0)),
        )
        .width(Length::Fixed(40.0))
        .height(Length::Fixed(40.0))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(0.05, 0.07, 0.10))),
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
    }).unwrap_or_else(|| app_icon_badge(&a.name, 40.0));

    let meta_cmd = row![
        icon(Icon::ArrowsHorizontal, FAINT, 11.0),
        text(ellipsize(&path_label, 50))
            .size(11)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(5)
    .align_y(Alignment::Center);

    let top_left = row![
        text(&a.name).size(15).color(INK).font(font::name_font()),
        profile_badge,
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let left_info = column![top_left, meta_cmd].spacing(6);

    let left_clickable = button(
        row![tile, left_info]
            .spacing(14)
            .align_y(Alignment::Center),
    )
    .padding(0)
    .width(Fill)
    .style(move |_t, _s| button::Style {
        background: None,
        border: Border::default(),
        text_color: INK,
        ..Default::default()
    })
    .on_press(Message::AppSelect(id));

    let right_column = column![
        row![
            iced::widget::Space::with_width(Length::Fill),
            status_indicator,
        ]
        .align_y(Alignment::Center),
        row![
            iced::widget::Space::with_width(Length::Fill),
            action_btn,
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(6)
    .align_x(Alignment::End);

    container(
        row![left_clickable, right_column]
            .spacing(12)
            .align_y(Alignment::Center),
    )
    .padding(Padding::from([12, 16]))
    .width(Fill)
    .style(move |_| card_style(selected))
    .into()
}

fn bottom_panel<'a>(app: &'a EnvBoxApp) -> Element<'a, Message> {
    let active_instances: Vec<_> = app
        .instances
        .list()
        .into_iter()
        .filter(|i| i.status == envbox_core::InstanceStatus::Running)
        .collect();

    if active_instances.is_empty() {
        return container(
            row![
                status_dot(FAINT),
                text("就绪 · 暂无运行中的实例 · 点击列表中的「运行」即可在隔离环境中启动")
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([8, 14]))
        .width(Fill)
        .style(inner_card_style)
        .into();
    }

    let rows = active_instances.into_iter().fold(
        Column::new().spacing(6),
        |col, inst| {
            let count = app.instances.child_count(inst.id).unwrap_or(0);
            let pname = app.profile_name(inst.profile_id);
            let uptime = format_uptime(inst.started_at);
            let app_name = app
                .applications
                .iter()
                .find(|a| a.id == inst.application_id)
                .map(|a| a.name.as_str())
                .unwrap_or("app");

            let info = row![
                status_dot(SUCCESS),
                text(format!("{}.exe", app_name.to_lowercase().replace(' ', "_")))
                    .size(13)
                    .color(INK)
                    .font(font::name_font()),
                text(format!("· PID {} · 子进程 {} · 已运行 {}", inst.root_pid, count, uptime))
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
                text(format!("· 配置文件: {}", pname))
                    .size(11)
                    .color(FAINT)
                    .font(font::ui_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .width(Fill);

            let open_loc = button(
                row![
                    icon(Icon::ExternalLink, INK_2, 11.0),
                    text("打开位置").size(11).color(INK_2).font(font::ui_font())
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([4, 10]))
            .style(secondary_btn)
            .on_press(Message::AppOpenLocation);

            let stop_btn = button(
                row![
                    icon(Icon::Stop, DANGER_TEXT, 10.0),
                    text("终止").size(11).color(DANGER_TEXT).font(font::ui_font())
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([4, 10]))
            .style(danger_btn)
            .on_press(Message::InstanceStop(inst.id));

            col.push(
                container(
                    row![info, row![open_loc, stop_btn].spacing(6).align_y(Alignment::Center)]
                        .spacing(10)
                        .align_y(Alignment::Center),
                )
                .padding(Padding::from([6, 12]))
                .width(Fill)
                .style(inner_card_style),
            )
        },
    );

    container(rows)
        .padding(Padding::from([4, 0]))
        .width(Fill)
        .into()
}
