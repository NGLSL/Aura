//! Right detail panel: application form tabs and profile form.

use iced::widget::{
    button, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{DnsChoice, LaunchKind, Message, NamedId};
use crate::theme::*;
use crate::widgets::{
    app_icon_badge, badge, danger_btn, field_label, form_row, kv_row, primary_btn, secondary_btn,
    status_dot, toggle,
};

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    match app.nav {
        crate::message::Nav::Apps => app_detail(app),
        crate::message::Nav::Profiles => profile_detail(app),
        _ => container(column![text("详情").size(14).color(MUTED)])
            .padding(18)
            .width(Length::Fixed(360.0))
            .height(Fill)
            .style(|_| panel_style(PANEL_RIGHT))
            .into(),
    }
}

fn app_detail(app: &EnvBoxApp) -> Element<'_, Message> {
    let selected = app.app_draft.id;
    let running = selected.map(|id| app.is_app_running(id)).unwrap_or(false);

    let app_name = if app.app_draft.name.is_empty() {
        "未选择应用"
    } else {
        app.app_draft.name.as_str()
    };

    let header_action: Element<_> = if let Some(id) = selected {
        if running {
            button(
                row![
                    icon(Icon::Stop, DANGER_TEXT, 11.0),
                    text("停止").size(12).color(DANGER_TEXT).font(font::name_font()),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([6, 12]))
            .style(danger_btn)
            .on_press(Message::AppStopId(id))
            .into()
        } else {
            button(
                row![
                    icon(Icon::Play, INK, 11.0),
                    text("启动").size(12).color(INK).font(font::name_font()),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([6, 14]))
            .style(primary_btn)
            .on_press(Message::AppRunId(id))
            .into()
        }
    } else {
        container(text("")).into()
    };

    let open_loc_btn = button(icon(Icon::ExternalLink, MUTED, 13.0))
        .padding(Padding::from([6, 8]))
        .style(secondary_btn)
        .on_press(Message::AppOpenLocation);

    let header_tile: Element<_> = selected
        .and_then(|id| app.app_icons.get(&id))
        .map(|png| {
            container(
                iced::widget::image(iced::widget::image::Handle::from_path(png.clone()))
                    .width(Length::Fixed(36.0))
                    .height(Length::Fixed(36.0)),
            )
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .style(|_| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(0.05, 0.07, 0.10))),
                border: iced::Border {
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
        .unwrap_or_else(|| app_icon_badge(&app.app_draft.name, 40.0));

    let header = row![
        header_tile,
        column![
            text(app_name)
                .size(17)
                .color(INK)
                .font(font::name_font()),
            row![
                status_dot(if running { SUCCESS } else { FAINT }),
                text(if running { "运行中" } else { "已停止" })
                    .size(12)
                    .color(if running { SUCCESS } else { MUTED })
                    .font(font::ui_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(2)
        .width(Fill),
        header_action,
        open_loc_btn,
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    // Profile options
    let profile_labels: Vec<NamedId> = app
        .profiles
        .iter()
        .map(|p| NamedId {
            name: p.name.clone(),
            id: p.id,
        })
        .collect();
    let selected_profile = profile_labels
        .iter()
        .find(|n| n.id == app.app_draft.profile_id)
        .cloned();

    let workdir_input = row![
        text_input("工作目录", &app.app_draft.work_dir)
            .on_input(Message::AppWorkDir)
            .padding(Padding::from([5, 8]))
            .style(input_style)
            .font(font::ui_font())
            .width(Fill),
        button(icon(Icon::Folder, MUTED, 13.0))
            .padding(Padding::from([5, 8]))
            .style(secondary_btn)
            .on_press(Message::AppBrowseWorkDir),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let form = column![
        field_label("基础配置"),
        form_row(
            "应用名称",
            text_input("应用名称", &app.app_draft.name)
                .on_input(Message::AppName)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        form_row(
            "启动方式",
            pick_list(
                LaunchKind::ALL,
                Some(app.app_draft.kind),
                Message::AppLaunchKind
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([5, 8]))
            .font(font::ui_font()),
        ),
        form_row(
            "命令 / 路径",
            text_input("可执行文件路径或命令行", &app.app_draft.path)
                .on_input(Message::AppPath)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        form_row(
            "参数",
            text_input("可选命令行参数 (例如 --flag)", &app.app_draft.args)
                .on_input(Message::AppArgs)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        form_row("工作目录", workdir_input),
        form_row(
            "环境 Profile",
            pick_list(profile_labels, selected_profile, |n: NamedId| Message::AppProfile(n.id))
                .style(pick_style)
                .menu_style(pick_menu)
                .padding(Padding::from([5, 8]))
                .font(font::ui_font()),
        ),
        field_label("环境行为"),
        row![
            text("子进程继承环境")
                .size(12)
                .color(INK_2)
                .font(font::ui_font())
                .width(Fill),
            toggle(app.app_draft.inherit, Message::AppInherit),
        ]
        .align_y(Alignment::Center),
        row![
            text("审计环境读取 (Audit Mode)")
                .size(12)
                .color(INK_2)
                .font(font::ui_font())
                .width(Fill),
            toggle(app.app_draft.audit, Message::AppAudit),
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(8);

    let form_actions = row![
        button(
            row![
                icon(Icon::Check, INK, 12.0),
                text("保存配置").size(13).color(INK).font(font::name_font())
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([8, 16]))
        .width(Fill)
        .style(primary_btn)
        .on_press(Message::AppSave),
        button(
            row![
                icon(Icon::Trash, DANGER_TEXT, 12.0),
                text("删除应用").size(13).color(DANGER_TEXT).font(font::ui_font())
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([8, 14]))
        .style(danger_btn)
        .on_press(Message::AppDelete),
    ]
    .spacing(8);

    let profile = app
        .profiles
        .iter()
        .find(|p| p.id == app.app_draft.profile_id);

    let profile_name = profile.map(|p| p.name.as_str()).unwrap_or("未选择配置文件");

    let profile_card = container(
        column![
            row![
                icon(Icon::Globe, ACCENT_LINE, 13.0),
                text(profile_name)
                    .size(13)
                    .color(INK)
                    .font(font::name_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            if let Some(p) = profile {
                let dns_label = match &p.dns.mode {
                    envbox_core::DnsMode::Host => "DNS: 宿主",
                    envbox_core::DnsMode::VirtualView => "DNS: 虚拟视图",
                };
                column![
                    row![
                        badge(&p.locale.region, ACCENT_BG, ACCENT_TEXT),
                        badge(&p.timezone.windows_id, BORDER, MUTED),
                        badge(dns_label, BORDER, MUTED),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                    kv_row("Region", &p.locale.region),
                    kv_row("Locale", &p.locale.locale_name),
                    kv_row("UI Language", &p.locale.ui_language),
                    kv_row("Timezone", &p.timezone.windows_id),
                    kv_row(
                        "DNS",
                        &if p.dns.servers.is_empty() {
                            "跟随宿主 DNS".to_string()
                        } else {
                            p.dns
                                .servers
                                .iter()
                                .map(|s| s.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    ),
                    kv_row(
                        "Environment",
                        &if p.environment.is_empty() {
                            "未额外定义变量".to_string()
                        } else {
                            p.environment
                                .iter()
                                .map(|(k, v)| format!("{k}={v}"))
                                .collect::<Vec<_>>()
                                .join("; ")
                        }
                    ),
                ]
                .spacing(4)
            } else {
                column![
                    text("未关联配置文件，请在上方指定默认配置文件")
                        .size(11)
                        .color(FAINT)
                        .font(font::ui_font())
                ]
                .spacing(4)
            }
        ]
        .spacing(6)
        .padding(10),
    )
    .width(Fill)
    .style(inner_card_style);

    container(
        scrollable(
            column![header, form, form_actions, profile_card]
                .spacing(12)
                .width(Fill),
        )
        .height(Fill)
        .style(dark_scrollable),
    )
    .padding(Padding::from([14, 16]))
    .width(Length::Fixed(350.0))
    .height(Fill)
    .style(|_| panel_style(PANEL_RIGHT))
    .into()
}

fn profile_detail(app: &EnvBoxApp) -> Element<'_, Message> {
    let is_new = app.profile_draft.id.is_none();
    let header = column![
        row![
            icon(Icon::Globe, ACCENT_LINE, 15.0),
            text(if is_new {
                "新建配置文件"
            } else if app.profile_draft.name.is_empty() {
                "未命名配置"
            } else {
                app.profile_draft.name.as_str()
            })
            .size(16)
            .color(INK)
            .font(font::name_font()),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        text(if is_new {
            "定义进程隔离参数 · 作用于进程树"
        } else {
            "编辑环境参数 · 保存后下次启动生效"
        })
        .size(11)
        .color(MUTED)
        .font(font::ui_font()),
    ]
    .spacing(2);

    let form = column![
        field_label("基础信息"),
        form_row(
            "配置名称",
            text_input("配置名称", &app.profile_draft.name)
                .on_input(Message::ProfileName)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        form_row(
            "Region",
            text_input("Region (ISO-2 代码，如 US/JP/CN)", &app.profile_draft.region)
                .on_input(Message::ProfileRegion)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        form_row(
            "Locale",
            text_input("Locale (如 en-US, ja-JP)", &app.profile_draft.locale)
                .on_input(Message::ProfileLocale)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        form_row(
            "UI Language",
            text_input("UI Language (如 en-US)", &app.profile_draft.ui)
                .on_input(Message::ProfileUi)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        field_label("时间系统"),
        form_row(
            "Windows 时区",
            pick_list(
                app.timezones.clone(),
                Some(app.profile_draft.tz.clone()),
                Message::ProfileTz
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([5, 8]))
            .font(font::ui_font()),
        ),
        form_row(
            "IANA 时区",
            text_input("IANA Timezone (如 America/New_York)", &app.profile_draft.tz_iana)
                .on_input(Message::ProfileTzIana)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        field_label("网络与 DNS"),
        form_row(
            "DNS 模式",
            pick_list(
                DnsChoice::ALL,
                Some(app.profile_draft.dns_mode),
                Message::ProfileDnsMode
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([5, 8]))
            .font(font::ui_font()),
        ),
        form_row(
            "DNS 服务器",
            text_input("逗号分隔，如 1.1.1.1, 8.8.8.8", &app.profile_draft.dns_servers)
                .on_input(Message::ProfileDnsServers)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
        ),
        field_label("环境变量"),
        text_input("KEY=VALUE;KEY2=VALUE2", &app.profile_draft.env)
            .on_input(Message::ProfileEnv)
            .padding(Padding::from([6, 8]))
            .style(input_style)
            .font(font::ui_font()),
    ]
    .spacing(8);

    let form_actions = row![
        button(
            row![
                icon(Icon::Check, INK, 12.0),
                text("保存配置").size(13).color(INK).font(font::name_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([8, 16]))
        .width(Fill)
        .style(primary_btn)
        .on_press(Message::ProfileSave),
        button(
            row![
                icon(Icon::Trash, DANGER_TEXT, 12.0),
                text("删除").size(13).color(DANGER_TEXT).font(font::ui_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([8, 14]))
        .style(danger_btn)
        .on_press(Message::ProfileDelete),
    ]
    .spacing(8);

    container(
        scrollable(
            column![header, form, form_actions]
                .spacing(12)
                .width(Fill),
        )
        .height(Fill)
        .style(dark_scrollable),
    )
    .padding(Padding::from([14, 16]))
    .width(Length::Fixed(350.0))
    .height(Fill)
    .style(|_| panel_style(PANEL_RIGHT))
    .into()
}
