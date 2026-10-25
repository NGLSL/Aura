//! Full-page instances list.

use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::{status_label, EnvBoxApp};
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{Message, WebRtcChoice};
use crate::theme::{self, FAINT, INK, INK_2, MUTED, SUCCESS};
use crate::theme::{TEXT_BODY, TEXT_CAPTION, TEXT_PAGE_TITLE, TEXT_SECTION};
use crate::widgets::{secondary_btn, short_id, status_dot};

pub fn view_full(app: &EnvBoxApp) -> Element<'_, Message> {
    use crate::message::Nav;
    use crate::widgets::{badge, danger_btn, primary_btn};

    let selected_name = app.instance_filter.and_then(|id| {
        app.applications
            .iter()
            .find(|application| application.id == id)
            .map(|application| application.name.as_str())
    });

    let instances: Vec<_> = app
        .instances
        .list()
        .into_iter()
        .filter(|instance| {
            !app.workspace_management
                .instances
                .iter()
                .any(|view| view.result.instance_id == instance.id)
                && app
                    .instance_filter
                    .map(|application_id| instance.application_id == application_id)
                    .unwrap_or(true)
        })
        .collect();
    let managed: Vec<_> = app
        .workspace_management
        .instances
        .iter()
        .filter(|view| {
            app.instance_filter
                .is_none_or(|id| view.result.application_id == id)
        })
        .collect();
    let active_count = instances
        .iter()
        .filter(|i| i.status == envbox_core::InstanceStatus::Running)
        .count()
        + managed
            .iter()
            .filter(|view| view.result.state == "Running")
            .count();

    let header = row![
        column![
            text("运行实例")
                .size(TEXT_PAGE_TITLE)
                .color(INK)
                .font(font::name_font()),
            text(format!(
                "共 {} 个实例 · 运行中 {} 个",
                instances.len() + managed.len(),
                active_count
            ))
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
        ]
        .spacing(4)
        .width(Fill),
        button(
            row![
                icon(Icon::Monitor, INK_2, 12.0),
                text("刷新实例状态")
                    .size(TEXT_BODY)
                    .color(INK_2)
                    .font(font::ui_font())
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([7, 14]))
        .style(secondary_btn)
        .on_press(Message::InstanceRefresh),
    ]
    .align_y(Alignment::Center);

    let filter_bar = instance_filter_bar(app, selected_name);

    let content: Element<_> = if instances.is_empty() {
        let launch_btn = button(
            row![
                icon(Icon::Play, INK, 11.0),
                text("前往应用列表启动")
                    .size(TEXT_BODY)
                    .color(INK)
                    .font(font::name_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([9, 20]))
        .style(primary_btn)
        .on_press(Message::Nav(Nav::Apps));

        if !managed.is_empty() {
            column![].into()
        } else if app.instance_filter.is_some() {
            instance_empty_state(
                "当前应用暂无实例",
                "可以切换到其他应用，或查看全部实例。",
                Some(
                    button(
                        text("查看全部")
                            .size(TEXT_BODY)
                            .color(INK)
                            .font(font::name_font()),
                    )
                    .padding(Padding::from([9, 20]))
                    .style(primary_btn)
                    .on_press(Message::InstanceFilter(None))
                    .into(),
                ),
            )
        } else {
            instance_empty_state(
                "暂无运行实例",
                "在「应用」页面中点击「运行」启动程序后，实例会显示在这里；点击“刷新实例状态”查看最新状态。",
                Some(launch_btn.into()),
            )
        }
    } else {
        let rows = instances
            .into_iter()
            .fold(column![].spacing(10), |col, inst| {
                let count = app.instances.child_count(inst.id).unwrap_or(0);
                let running = inst.status == envbox_core::InstanceStatus::Running;
                let application = app
                    .applications
                    .iter()
                    .find(|a| a.id == inst.application_id);
                let app_name = application.map(|a| a.name.as_str()).unwrap_or("未知应用");
                let profile_label = if inst.profile_id.is_nil() {
                    "宿主".to_string()
                } else {
                    app.profile_name(inst.profile_id)
                };

                let status_badge = if running {
                    badge("● 运行中", theme::SUCCESS_BG, theme::SUCCESS_TEXT)
                } else {
                    badge(status_label(inst.status), theme::BORDER, MUTED)
                };

                // A host run has no Profile, so its WebRTC state is not shown as
                // if it came from a profile.
                let webrtc_badge = if inst.profile_id.is_nil() {
                    None
                } else {
                    app.profiles
                        .iter()
                        .find(|p| p.id == inst.profile_id)
                        .map(|p| {
                            let active = p.browser.webrtc != envbox_core::WebRtcPolicy::Host;
                            badge(
                                WebRtcChoice::from_policy(&p.browser.webrtc).short_label(),
                                if active {
                                    theme::ACCENT_BG
                                } else {
                                    theme::BORDER
                                },
                                if active { theme::ACCENT_TEXT } else { MUTED },
                            )
                        })
                };

                let mut name_row = row![
                    text(app_name)
                        .size(TEXT_SECTION)
                        .color(INK)
                        .font(font::name_font()),
                    text(format!("PID {}", inst.root_pid))
                        .size(TEXT_CAPTION)
                        .color(theme::ACCENT_TEXT)
                        .font(font::ui_font()),
                    status_badge,
                    badge(&profile_label, theme::ACCENT_BG, theme::ACCENT_TEXT),
                ]
                .spacing(10)
                .align_y(Alignment::Center);
                if let Some(wb) = webrtc_badge {
                    name_row = name_row.push(wb);
                }

                let stop_btn = if running {
                    button(
                        row![
                            icon(Icon::Stop, theme::DANGER_TEXT, 11.0),
                            text("终止进程树")
                                .size(TEXT_BODY)
                                .color(theme::DANGER_TEXT)
                                .font(font::ui_font())
                        ]
                        .spacing(5)
                        .align_y(Alignment::Center),
                    )
                    .padding(Padding::from([6, 14]))
                    .style(danger_btn)
                    .on_press(Message::InstanceStop(inst.id))
                } else {
                    button(text("已结束").size(TEXT_BODY).color(FAINT))
                        .padding(Padding::from([6, 14]))
                        .style(secondary_btn)
                };

                let mut actions = row![].spacing(6).align_y(Alignment::Center);
                if application.is_some_and(|a| {
                    matches!(&a.launch, envbox_core::LaunchTarget::Executable { .. })
                }) {
                    actions = actions.push(
                        button(
                            row![
                                icon(Icon::ExternalLink, INK_2, 11.0),
                                text("打开位置")
                                    .size(TEXT_BODY)
                                    .color(INK_2)
                                    .font(font::ui_font()),
                            ]
                            .spacing(4)
                            .align_y(Alignment::Center),
                        )
                        .padding(Padding::from([4, 10]))
                        .style(secondary_btn)
                        .on_press(Message::AppOpenLocationId(inst.application_id)),
                    );
                }
                actions = actions.push(stop_btn);

                let card = container(
                    row![
                        status_dot(if running { SUCCESS } else { FAINT }),
                        column![
                            name_row,
                            row![
                                text(format!("衍生子进程: {} 个", count))
                                    .size(TEXT_CAPTION)
                                    .color(MUTED)
                                    .font(font::ui_font()),
                                text("·").size(TEXT_CAPTION).color(FAINT),
                                text(format!("实例 ID: {}", short_id(inst.id)))
                                    .size(TEXT_CAPTION)
                                    .color(FAINT)
                                    .font(font::ui_font()),
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                        ]
                        .spacing(4)
                        .width(Fill),
                        actions,
                    ]
                    .spacing(16)
                    .align_y(Alignment::Center),
                )
                .padding(Padding::from([14, 18]))
                .width(Fill)
                .style(theme::inner_card_style);

                col.push(card)
            });

        column![
            text("本次会话实例")
                .size(TEXT_SECTION)
                .color(INK)
                .font(font::name_font()),
            rows
        ]
        .spacing(10)
        .into()
    };

    let content = scrollable(column![super::workspaces::view(app), content].spacing(16))
        .height(Fill)
        .style(theme::dark_scrollable);
    column![header, filter_bar, content]
        .spacing(12)
        .width(Fill)
        .height(Fill)
        .into()
}

fn instance_filter_bar<'a>(
    app: &'a EnvBoxApp,
    selected_name: Option<&'a str>,
) -> Element<'a, Message> {
    let all_selected = app.instance_filter.is_none();
    let mut filters = row![button(text("全部").size(TEXT_BODY).font(font::ui_font()))
        .padding(Padding::from([5, 10]))
        .style(move |_theme, _status| theme::tab_btn_style(all_selected))
        .on_press(Message::InstanceFilter(None)),]
    .spacing(4)
    .align_y(Alignment::Center);

    for application in &app.applications {
        let selected = app.instance_filter == Some(application.id);
        let count = app.running_count(application.id);
        filters = filters.push(
            button(
                row![
                    text(&application.name)
                        .size(TEXT_BODY)
                        .font(font::ui_font()),
                    text(format!("{count}"))
                        .size(TEXT_CAPTION)
                        .color(if selected { theme::ACCENT_TEXT } else { FAINT })
                        .font(font::ui_font()),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([5, 10]))
            .style(move |_theme, _status| theme::tab_btn_style(selected))
            .on_press(Message::InstanceFilter(Some(application.id))),
        );
    }

    let label = selected_name
        .map(|name| format!("当前筛选：{name}"))
        .unwrap_or_else(|| {
            if app.instance_filter.is_some() {
                "当前筛选：应用已删除".into()
            } else {
                "查看全部应用的实例".into()
            }
        });

    container(
        column![
            text(label)
                .size(TEXT_CAPTION)
                .color(MUTED)
                .font(font::ui_font()),
            scrollable(filters)
                .direction(scrollable::Direction::Horizontal(Default::default()))
                .width(Fill)
                .height(Length::Fixed(42.0))
                .style(theme::dark_scrollable),
        ]
        .spacing(4),
    )
    .padding(Padding::from([7, 10]))
    .width(Fill)
    .style(theme::inner_card_style)
    .into()
}

// Unlike the full-page empty_state helper, this body is intrinsic-height:
// it shares a vertical scroller with management state and must not request Fill.
fn instance_empty_state<'a>(
    title: &'static str,
    description: &'static str,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut body = column![
        icon(Icon::Monitor, theme::ACCENT_LINE, 28.0),
        text(title)
            .size(TEXT_SECTION)
            .color(INK)
            .font(font::name_font()),
        text(description)
            .size(TEXT_BODY)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(12)
    .align_x(Alignment::Center);
    if let Some(action) = action {
        body = body.push(container(action).padding(Padding::from([6, 0])));
    }
    container(body)
        .padding(Padding::from([56, 24]))
        .width(Fill)
        .center_x(Fill)
        .into()
}
