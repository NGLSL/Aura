//! Full-page instances list.

use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::{status_label, EnvBoxApp};
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{Message, WebRtcChoice};
use crate::theme::{self, FAINT, INK, INK_2, MUTED, SUCCESS};
use crate::widgets::{secondary_btn, short_id, status_dot};

pub fn view_full(app: &EnvBoxApp) -> Element<'_, Message> {
    use crate::message::Nav;
    use crate::widgets::{badge, danger_btn, empty_state, primary_btn};

    let active_count = app
        .instances
        .list()
        .into_iter()
        .filter(|i| i.status == envbox_core::InstanceStatus::Running)
        .count();

    let header = row![
        column![
            text("运行实例").size(24).color(INK).font(font::name_font()),
            text(format!(
                "实时监控 · 活跃运行实例 {} 个 · 受控进程树及其子进程生命周期",
                active_count
            ))
            .size(12)
            .color(MUTED)
            .font(font::ui_font()),
        ]
        .spacing(4)
        .width(Fill),
        button(
            row![
                icon(Icon::Monitor, INK_2, 12.0),
                text("刷新实例状态").size(12).color(INK_2).font(font::ui_font())
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([7, 14]))
        .style(secondary_btn)
        .on_press(Message::InstanceRefresh),
    ]
    .align_y(Alignment::Center);

    let instances = app.instances.list();

    let content: Element<_> = if instances.is_empty() {
        let launch_btn = button(
            row![
                icon(Icon::Play, INK, 11.0),
                text("前往应用列表启动")
                    .size(13)
                    .color(INK)
                    .font(font::name_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([9, 20]))
        .style(primary_btn)
        .on_press(Message::Nav(Nav::Apps));

        empty_state(
            Icon::Monitor,
            "暂无活跃的运行实例",
            "在「应用」页面中点击「运行」启动程序后，完整的受控进程树、PID 与虚拟化环境状态将在此实时呈现。",
            Some(launch_btn.into()),
        )
    } else {
        let rows = instances.into_iter().fold(
            column![].spacing(10),
            |col, inst| {
                let count = app.instances.child_count(inst.id).unwrap_or(0);
                let pname = app.profile_name(inst.profile_id);
                let running = inst.status == envbox_core::InstanceStatus::Running;
                let app_name = app
                    .applications
                    .iter()
                    .find(|a| a.id == inst.application_id)
                    .map(|a| a.name.as_str())
                    .unwrap_or("应用实例");

                let status_badge = if running {
                    badge("● 运行中", theme::SUCCESS_BG, theme::SUCCESS_TEXT)
                } else {
                    badge(status_label(inst.status), theme::BORDER, MUTED)
                };

                let webrtc = app
                    .profiles
                    .iter()
                    .find(|p| p.id == inst.profile_id)
                    .map(|p| p.browser.webrtc);
                let webrtc_badge = webrtc.map(|w| {
                    let active = w != envbox_core::WebRtcPolicy::Host;
                    badge(
                        WebRtcChoice::from_policy(&w).short_label(),
                        if active { theme::ACCENT_BG } else { theme::BORDER },
                        if active { theme::ACCENT_TEXT } else { MUTED },
                    )
                });

                let stop_btn = if running {
                    button(
                        row![
                            icon(Icon::Stop, theme::DANGER_TEXT, 11.0),
                            text("终止进程树").size(12).color(theme::DANGER_TEXT).font(font::ui_font())
                        ]
                        .spacing(5)
                        .align_y(Alignment::Center),
                    )
                    .padding(Padding::from([6, 14]))
                    .style(danger_btn)
                    .on_press(Message::InstanceStop(inst.id))
                } else {
                    button(text("已结束").size(12).color(FAINT))
                        .padding(Padding::from([6, 14]))
                        .style(secondary_btn)
                };

                let mut name_row = row![
                    text(format!("{}.exe", app_name.to_lowercase().replace(' ', "_")))
                        .size(15)
                        .color(INK)
                        .font(font::name_font()),
                    text(format!("PID {}", inst.root_pid))
                        .size(12)
                        .color(theme::ACCENT_TEXT)
                        .font(font::ui_font()),
                    status_badge,
                    badge(&pname, theme::ACCENT_BG, theme::ACCENT_TEXT),
                ]
                .spacing(10)
                .align_y(Alignment::Center);
                if let Some(wb) = webrtc_badge {
                    name_row = name_row.push(wb);
                }

                let card = container(
                    row![
                        status_dot(if running { SUCCESS } else { FAINT }),
                        column![
                            name_row,
                            row![
                                text(format!("衍生子进程: {} 个", count))
                                    .size(12)
                                    .color(MUTED)
                                    .font(font::ui_font()),
                                text("·")
                                    .size(12)
                                    .color(FAINT),
                                text(format!("实例 ID: {}", short_id(inst.id)))
                                    .size(11)
                                    .color(FAINT)
                                    .font(font::ui_font()),
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                        ]
                        .spacing(4)
                        .width(Fill),
                        stop_btn,
                    ]
                    .spacing(16)
                    .align_y(Alignment::Center),
                )
                .padding(Padding::from([14, 18]))
                .width(Fill)
                .style(theme::inner_card_style);

                col.push(card)
            },
        );

        scrollable(rows)
            .height(Fill)
            .style(theme::dark_scrollable)
            .into()
    };

    column![header, content]
        .spacing(16)
        .width(Fill)
        .height(Fill)
        .into()
}
