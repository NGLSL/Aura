use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_input, tooltip,
};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{browser_guarantee_label, DnsChoice, Message, WebRtcChoice};
use crate::theme::*;
use crate::widgets::{
    danger_btn, field_label, form_row, kv_row, primary_btn, searchable_select, secondary_btn,
};

use super::section_title;

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

    container(
        scrollable(
            container(column![header, content].spacing(20))
                .width(Fill)
                .max_width(900)
                .padding(Padding::from([8, 20])),
        )
        .height(Fill)
        .style(dark_scrollable),
    )
    .padding(Padding::from([14, 16]))
    .width(Fill)
    .height(Fill)
    .style(|_| panel_style(PANEL_RIGHT))
    .into()
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
            container(
                column![
                    field_label("身份信息视图"),
                    kv_row(
                        "主机名",
                        p.identity.computer_name.as_deref().unwrap_or("跟随宿主")
                    ),
                    kv_row(
                        "用户名",
                        p.identity.user_name.as_deref().unwrap_or("跟随宿主")
                    ),
                    kv_row(
                        "MAC",
                        p.identity.mac_address.as_deref().unwrap_or("跟随宿主")
                    ),
                    kv_row(
                        "MachineGuid",
                        p.identity.machine_guid.as_deref().unwrap_or("跟随宿主")
                    ),
                    text("仅影响支持的读取入口；真实账户、网卡和系统注册表保持原值。")
                        .size(10)
                        .color(FAINT)
                        .font(font::ui_font()),
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
            text("身份、IANA、DNS、WebRTC 和变量")
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
                row![
                    field_label("身份信息视图 · 留空跟随宿主"),
                    tooltip(icon(Icon::Info, MUTED, 12.0),
                        container(column![
                            text("提供 Win32 主机名 / 用户名、网卡信息和 MachineGuid 注册表读取视图。真实账户、网卡和系统安装标识保持原值；CPU、GPU、磁盘身份暂不覆盖。").size(12).font(font::ui_font()),
                            text("GetUserNameEx、WMI、Native Registry API 和设备 IOCTL 暂不覆盖。").size(12).font(font::ui_font()),
                        ].spacing(8)).width(320).padding(12),
                        tooltip::Position::Left,
                    ).style(inner_card_style),
                ].spacing(8).align_y(Alignment::Center),
                form_row(
                    "主机名",
                    text_input("1–15 位 ASCII 主机名", &app.profile_draft.identity.computer_name)
                        .on_input(Message::ProfileComputerName)
                        .padding(Padding::from([5, 8]))
                        .style(input_style)
                        .font(font::ui_font()),
                ),
                form_row(
                    "用户名",
                    text_input("字母、数字、点、下划线或连字符", &app.profile_draft.identity.user_name)
                        .on_input(Message::ProfileUserName)
                        .padding(Padding::from([5, 8]))
                        .style(input_style)
                        .font(font::ui_font()),
                ),
                form_row(
                    "MAC",
                    row![
                        text_input("02:AA:BB:CC:DD:EE", &app.profile_draft.identity.mac_address)
                            .on_input(Message::ProfileMacAddress)
                            .padding(Padding::from([5, 8]))
                            .style(input_style)
                            .font(font::ui_font())
                            .width(Fill),
                        button(text("生成").size(12).font(font::ui_font()))
                            .padding(Padding::from([6, 10]))
                            .style(secondary_btn)
                            .on_press(Message::ProfileMacAddressGenerate),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ),
                form_row(
                    "MachineGuid",
                    row![
                        text_input("Windows 安装标识 UUID", &app.profile_draft.identity.machine_guid)
                            .on_input(Message::ProfileMachineGuid)
                            .padding(Padding::from([5, 8]))
                            .style(input_style)
                            .font(font::ui_font())
                            .width(Fill),
                        button(text("生成").size(12).font(font::ui_font()))
                            .padding(Padding::from([6, 10]))
                            .style(secondary_btn)
                            .on_press(Message::ProfileMachineGuidGenerate),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ),
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
    let mut explanation = column![
        text("Strict：受支持的 DNS API 解析失败时，禁止回退宿主 DNS。")
            .size(12)
            .font(font::ui_font()),
        text("UDP、TCP、DoT、DoH 按列表顺序尝试，不自动添加上游。Host 模式使用宿主 DNS。")
            .size(12)
            .font(font::ui_font()),
        text("应用自带 DNS/DoH/DoT/DoQ 可能绕过这些 API；浏览器 renderer 的信息覆盖需单独验证。")
            .size(12)
            .font(font::ui_font()),
    ]
    .spacing(8);
    if editor
        .upstreams
        .iter()
        .any(envbox_core::DnsUpstream::is_plaintext)
    {
        explanation = explanation.push(
            text("包含 UDP/TCP 明文上游；前面的加密上游失败后可能发送明文查询。")
                .size(12)
                .font(font::ui_font()),
        );
    }
    let help = tooltip(
        row![
            text("说明").size(12).color(MUTED).font(font::ui_font()),
            icon(Icon::Info, MUTED, 12.0)
        ]
        .spacing(4)
        .align_y(Alignment::Center),
        container(explanation)
            .width(320)
            .padding(12)
            .style(inner_card_style),
        tooltip::Position::Left,
    )
    .gap(8)
    .style(inner_card_style);
    let mut content = column![row![
        checkbox("Strict：禁止回退宿主 DNS", editor.strict)
            .on_toggle(Message::ProfileDnsStrict)
            .text_size(12)
            .width(Fill),
        help,
    ]
    .spacing(8)
    .align_y(Alignment::Center),]
    .spacing(8);
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
                text(format!("{} · {}", index + 1, upstream.label()))
                    .size(12)
                    .width(Fill),
                button("↑")
                    .style(secondary_btn)
                    .on_press(Message::ProfileDnsMove(index, true)),
                button("↓")
                    .style(secondary_btn)
                    .on_press(Message::ProfileDnsMove(index, false)),
                button("移除")
                    .style(secondary_btn)
                    .on_press(Message::ProfileDnsRemove(index)),
            ]
            .spacing(4),
        );
    }
    content = content.push(
        pick_list(
            TransportChoice::ALL,
            Some(editor.draft.transport),
            Message::ProfileDnsTransport,
        )
        .style(pick_style)
        .menu_style(pick_menu)
        .text_size(12),
    );
    if editor.draft.transport == TransportChoice::Doh {
        content = content.push(
            pick_list(
                envbox_core::DnsTlsRevocation::ALL,
                Some(editor.draft.tls_revocation),
                Message::ProfileDnsTlsRevocation,
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([5, 8]))
            .font(font::ui_font())
            .text_size(12),
        );
        content = content.push(
            text_input("https://resolver.example/dns-query", &editor.draft.url)
                .on_input(Message::ProfileDnsUrl)
                .style(input_style)
                .padding(8)
                .font(font::ui_font()),
        );
        content = content.push(tooltip(
            field_label("连接 IP · 可选"),
            container(text("留空时通过 Profile 中可直接连接的上游解析 DoH 服务域名；没有可用上游时解析失败，不调用宿主 DNS。手填 IP 会覆盖自动解析。").size(12).font(font::ui_font()))
                .width(320).padding(12),
            tooltip::Position::Left,
        ).style(inner_card_style));
        content = content.push(
            text_input("留空自动解析；或填写 IP，逗号分隔", &editor.draft.bootstrap)
                .on_input(Message::ProfileDnsBootstrap)
                .style(input_style)
                .padding(8)
                .font(font::ui_font()),
        );
    } else {
        content = content.push(
            text_input("literal IP，例如 1.1.1.1", &editor.draft.address)
                .on_input(Message::ProfileDnsAddress)
                .style(input_style)
                .padding(8)
                .font(font::ui_font()),
        );
        content = content.push(
            text_input("端口", &editor.draft.port)
                .on_input(Message::ProfileDnsPort)
                .style(input_style)
                .padding(8)
                .font(font::ui_font()),
        );
        if editor.draft.transport == TransportChoice::Dot {
            content = content.push(
                text_input(
                    "证书身份，例如 cloudflare-dns.com",
                    &editor.draft.server_name,
                )
                .on_input(Message::ProfileDnsServerName)
                .style(input_style)
                .padding(8)
                .font(font::ui_font()),
            );
        }
    }
    if let Some(error) = editor.draft_error() {
        content = content.push(text(error).size(12).color(DANGER).font(font::ui_font()));
    }
    content
        .push(
            button("添加上游")
                .style(secondary_btn)
                .on_press(Message::ProfileDnsAdd),
        )
        .into()
}
