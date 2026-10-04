use iced::widget::{button, checkbox, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{browser_guarantee_label, DnsChoice, Message, WebRtcChoice};
use crate::theme::*;
use crate::widgets::{
    danger_btn, field_label, form_row, kv_row, primary_btn, searchable_select, secondary_btn,
};

use super::{detail_shell, section_title};

pub(super) fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let is_new = app.profile_draft.id.is_none();
    let editing = app.profile_edit_mode || is_new;
    let title = if is_new {
        "新建环境配置"
    } else if app.profile_draft.name.is_empty() {
        "未命名配置"
    } else {
        app.profile_draft.name.as_str()
    };

    let header_action: Element<'_, Message> = if is_new {
        container(text("新建").size(12).color(ACCENT_TEXT)).into()
    } else if editing {
        button(text("取消").size(12).font(font::ui_font()))
            .padding(Padding::from([6, 10]))
            .style(secondary_btn)
            .on_press(Message::ProfileEditCancel)
            .into()
    } else {
        button(text("编辑").size(12).font(font::ui_font()))
            .padding(Padding::from([6, 10]))
            .style(secondary_btn)
            .on_press(Message::ProfileEdit)
            .into()
    };

    let header = row![
        row![
            icon(Icon::Globe, ACCENT_LINE, 15.0),
            column![
                text(title).size(16).color(INK).font(font::name_font()),
                text(if is_new {
                    "填写一组应用可以复用的环境设置"
                } else {
                    "启动应用时按这组设置准备进程环境"
                })
                .size(11)
                .color(MUTED)
                .font(font::ui_font()),
            ]
            .spacing(2),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill),
        header_action,
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let content = if editing {
        profile_editor(app)
    } else {
        profile_overview(app)
    };

    detail_shell(column![header, content].spacing(12))
}

fn profile_overview(app: &EnvBoxApp) -> Element<'_, Message> {
    let p = match app
        .profile_draft
        .id
        .and_then(|id| app.profiles.iter().find(|p| p.id == id))
    {
        Some(p) => p,
        None => {
            return container(
                column![
                    text("还没有选择环境配置")
                        .size(15)
                        .color(INK)
                        .font(font::name_font()),
                    text("从左侧选择环境配置，或点击“新建环境配置”。")
                        .size(12)
                        .color(MUTED)
                        .font(font::ui_font()),
                ]
                .spacing(8),
            )
            .padding(14)
            .width(Fill)
            .style(inner_card_style)
            .into()
        }
    };

    let dns = if p.dns.mode == envbox_core::DnsMode::Host {
        "跟随系统 DNS".to_string()
    } else {
        p.dns
            .effective_upstreams()
            .iter()
            .map(|s| s.label())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let webrtc = WebRtcChoice::from_policy(&p.browser.webrtc);

    container(
        column![
            section_title("配置概览", "保存后，使用此配置启动的应用会读取这些值"),
            container(
                column![
                    kv_row("Region", &p.locale.region),
                    kv_row("Locale", &p.locale.locale_name),
                    kv_row("界面语言", &p.locale.ui_language),
                    kv_row("Windows 时区", &p.timezone.windows_id),
                ]
                .spacing(7)
                .padding(10),
            )
            .width(Fill)
            .style(inner_card_style),
            container(
                column![
                    field_label("高级设置"),
                    kv_row(
                        "IANA 时区",
                        if p.timezone.iana_id.is_empty() {
                            "跟随 Windows 时区"
                        } else {
                            &p.timezone.iana_id
                        },
                    ),
                    kv_row("DNS", &format!("{} · {}", dns_mode_label(&p.dns.mode), dns)),
                    kv_row(
                        "WebRTC",
                        &format!("{} · {}", webrtc, browser_guarantee_label(p.browser.webrtc)),
                    ),
                    kv_row("环境变量", &format!("{} 项", p.environment.len())),
                ]
                .spacing(7)
                .padding(10),
            )
            .width(Fill)
            .style(inner_card_style),
            button(text("编辑配置").size(12).color(INK_2).font(font::ui_font()))
                .padding(Padding::from([7, 12]))
                .style(secondary_btn)
                .on_press(Message::ProfileEdit),
            button(
                text("删除环境配置")
                    .size(11)
                    .color(DANGER_TEXT)
                    .font(font::ui_font())
            )
            .padding(Padding::from([6, 9]))
            .style(danger_btn)
            .on_press(Message::ProfileDelete),
        ]
        .spacing(10),
    )
    .width(Fill)
    .into()
}

fn profile_editor(app: &EnvBoxApp) -> Element<'_, Message> {
    let region_opts = app.combo_options(crate::message::ComboField::Region);
    let locale_opts = app.combo_options(crate::message::ComboField::Locale);
    let ui_opts = app.combo_options(crate::message::ComboField::Ui);
    let tz_opts = app.combo_options(crate::message::ComboField::Timezone);
    let tz_iana_opts = app.combo_options(crate::message::ComboField::TimezoneIana);

    let basic = column![
        field_label("基本信息"),
        form_row(
            "配置名称",
            text_input("配置名称", &app.profile_draft.name)
                .on_input(Message::ProfileName)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font()),
        ),
        form_row(
            "Region",
            searchable_select(
                crate::message::ComboField::Region,
                &app.profile_draft.region,
                "选择 Region",
                app.open_combo == Some(crate::message::ComboField::Region),
                &app.combo_query,
                &region_opts,
                Message::ComboQuery,
                Message::ComboPick,
            ),
        ),
        form_row(
            "Locale",
            searchable_select(
                crate::message::ComboField::Locale,
                &app.profile_draft.locale,
                "选择 Locale",
                app.open_combo == Some(crate::message::ComboField::Locale),
                &app.combo_query,
                &locale_opts,
                Message::ComboQuery,
                Message::ComboPick,
            ),
        ),
        form_row(
            "界面语言",
            searchable_select(
                crate::message::ComboField::Ui,
                &app.profile_draft.ui,
                "选择界面语言",
                app.open_combo == Some(crate::message::ComboField::Ui),
                &app.combo_query,
                &ui_opts,
                Message::ComboQuery,
                Message::ComboPick,
            ),
        ),
        form_row(
            "Windows 时区",
            searchable_select(
                crate::message::ComboField::Timezone,
                &app.profile_draft.tz,
                "选择 Windows 时区",
                app.open_combo == Some(crate::message::ComboField::Timezone),
                &app.combo_query,
                &tz_opts,
                Message::ComboQuery,
                Message::ComboPick,
            ),
        ),
    ]
    .spacing(8);

    let advanced_toggle = button(
        row![
            icon(
                if app.profile_advanced {
                    Icon::ChevronDown
                } else {
                    Icon::MoreHorizontal
                },
                ACCENT_LINE,
                11.0,
            ),
            text("高级环境设置")
                .size(12)
                .color(INK_2)
                .font(font::name_font()),
            text("IANA、DNS、WebRTC 和变量")
                .size(11)
                .color(MUTED)
                .font(font::ui_font()),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding(Padding::from([8, 10]))
    .style(secondary_btn)
    .on_press(Message::ProfileAdvancedToggle);

    let advanced: Element<_> = if app.profile_advanced {
        container(
            column![
                form_row(
                    "IANA 时区",
                    searchable_select(
                        crate::message::ComboField::TimezoneIana,
                        &app.profile_draft.tz_iana,
                        "选择 IANA 时区",
                        app.open_combo == Some(crate::message::ComboField::TimezoneIana),
                        &app.combo_query,
                        &tz_iana_opts,
                        Message::ComboQuery,
                        Message::ComboPick,
                    ),
                ),
                form_row(
                    "DNS 模式",
                    pick_list(
                        DnsChoice::ALL,
                        Some(app.profile_draft.dns_mode),
                        Message::ProfileDnsMode,
                    )
                    .style(pick_style)
                    .menu_style(pick_menu)
                    .padding(Padding::from([5, 8]))
                    .font(font::ui_font()),
                ),
                form_row("DNS 上游顺序", dns_editor_view(app),),
                form_row(
                    "WebRTC",
                    pick_list(
                        WebRtcChoice::ALL,
                        Some(app.profile_draft.webrtc),
                        Message::ProfileWebRtc,
                    )
                    .style(pick_style)
                    .menu_style(pick_menu)
                    .padding(Padding::from([5, 8]))
                    .font(font::ui_font()),
                ),
                text("Host 不修改浏览器行为；严格模式会同时启用运行时网络保护。")
                    .size(10)
                    .color(FAINT)
                    .font(font::ui_font()),
                form_row(
                    "环境变量",
                    text_input("KEY=VALUE;KEY2=VALUE2", &app.profile_draft.env)
                        .on_input(Message::ProfileEnv)
                        .padding(Padding::from([6, 8]))
                        .style(input_style)
                        .font(font::ui_font()),
                ),
            ]
            .spacing(8)
            .padding(10),
        )
        .width(Fill)
        .style(inner_card_style)
        .into()
    } else {
        container(text("")).into()
    };

    let save = button(
        row![
            icon(Icon::Check, INK, 12.0),
            text("保存").size(13).color(INK).font(font::name_font()),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 16]))
    .width(Fill)
    .style(primary_btn)
    .on_press(Message::ProfileSave);
    let cancel = button(text("取消").size(13).color(INK_2).font(font::ui_font()))
        .padding(Padding::from([8, 13]))
        .style(secondary_btn)
        .on_press(Message::ProfileEditCancel);
    let delete: Element<_> = if app.profile_draft.id.is_some() {
        button(
            text("删除")
                .size(11)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        )
        .padding(Padding::from([6, 9]))
        .style(danger_btn)
        .on_press(Message::ProfileDelete)
        .into()
    } else {
        container(text("")).into()
    };

    column![
        section_title("编辑配置", "保存后，新的实例会使用更新后的设置"),
        basic,
        advanced_toggle,
        advanced,
        row![save, cancel, delete]
            .spacing(8)
            .align_y(Alignment::Center),
    ]
    .spacing(10)
    .into()
}
fn dns_mode_label(mode: &envbox_core::DnsMode) -> &'static str {
    match mode {
        envbox_core::DnsMode::Host => "宿主",
        envbox_core::DnsMode::VirtualView => "虚拟视图",
    }
}

fn dns_editor_view(app: &EnvBoxApp) -> Element<'_, Message> {
    use crate::app::dns_editor::TransportChoice;
    let editor = &app.profile_draft.dns_editor;
    let mut content = column![
        checkbox("Strict：全部上游失败时禁止回退宿主 DNS", editor.strict)
            .on_toggle(Message::ProfileDnsStrict),
        text("按列表顺序尝试，不自动添加上游。Host 模式单独使用宿主 DNS。"),
    ]
    .spacing(8);
    if editor
        .upstreams
        .iter()
        .any(envbox_core::DnsUpstream::is_plaintext)
    {
        content = content.push(text(
            "包含 UDP/TCP 明文上游；前面的加密上游失败后可能发送明文查询。",
        ));
    }
    if editor
        .to_profile(app.profile_draft.dns_mode.to_mode())
        .ok()
        .is_some_and(|dns| dns.validate_runtime_support().is_err())
    {
        content = content.push(text(
            "配置可以保存；当前 Runtime 不支持此协议、端口或 non-strict 组合，启动会拒绝。",
        ));
    }
    for (index, upstream) in editor.upstreams.iter().enumerate() {
        content = content.push(
            row![
                text(format!("{} · {}", index + 1, upstream.label())).width(Fill),
                button("↑").on_press(Message::ProfileDnsMove(index, true)),
                button("↓").on_press(Message::ProfileDnsMove(index, false)),
                button("移除").on_press(Message::ProfileDnsRemove(index)),
            ]
            .spacing(4),
        );
    }
    content = content.push(pick_list(
        TransportChoice::ALL,
        Some(editor.draft.transport),
        Message::ProfileDnsTransport,
    ));
    if editor.draft.transport == TransportChoice::Doh {
        content = content.push(
            text_input("https://resolver.example/dns-query", &editor.draft.url)
                .on_input(Message::ProfileDnsUrl),
        );
        content = content.push(
            text_input("显式 bootstrap IP，逗号分隔", &editor.draft.bootstrap)
                .on_input(Message::ProfileDnsBootstrap),
        );
    } else {
        content = content.push(
            text_input("literal IP，例如 1.1.1.1", &editor.draft.address)
                .on_input(Message::ProfileDnsAddress),
        );
        content =
            content.push(text_input("端口", &editor.draft.port).on_input(Message::ProfileDnsPort));
        if editor.draft.transport == TransportChoice::Dot {
            content = content.push(
                text_input(
                    "证书身份，例如 cloudflare-dns.com",
                    &editor.draft.server_name,
                )
                .on_input(Message::ProfileDnsServerName),
            );
        }
    }
    content
        .push(button("添加上游").on_press(Message::ProfileDnsAdd))
        .into()
}
