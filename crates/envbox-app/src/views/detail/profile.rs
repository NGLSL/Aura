use iced::widget::{button, column, container, pick_list, row, text, text_input, tooltip};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{browser_guarantee_label, DnsChoice, Message, WebRtcChoice};
use crate::theme::*;
use crate::widgets::{
    danger_btn, field_label, form_field, form_section, kv_row, primary_btn, searchable_select,
    secondary_btn,
};

use super::{detail_editor_shell, detail_shell, section_title};

mod dns;
mod timezone;

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
        container(text("新建").size(TEXT_BODY).color(ACCENT_TEXT)).into()
    } else if editing {
        container(text("")).into()
    } else {
        button(text("编辑").size(TEXT_BODY).font(font::ui_font()))
            .padding(Padding::from([8, 10]))
            .style(secondary_btn)
            .on_press(Message::ProfileEdit)
            .into()
    };

    let header = row![
        row![
            icon(Icon::Globe, ACCENT_LINE, 15.0),
            column![
                text(title)
                    .size(TEXT_TITLE)
                    .color(INK)
                    .font(font::name_font()),
                text(if is_new {
                    "填写一组应用可以复用的环境设置"
                } else {
                    "启动应用时按这组设置准备进程环境"
                })
                .size(TEXT_CAPTION)
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

    if editing {
        let (body, actions) = profile_editor(app);
        detail_editor_shell(column![header, body].spacing(20), actions)
    } else {
        detail_shell(column![header, profile_overview(app)].spacing(20))
    }
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
                        .size(TEXT_SECTION)
                        .color(INK)
                        .font(font::name_font()),
                    text("从左侧选择环境配置，或点击“新建环境配置”。")
                        .size(TEXT_BODY)
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

    let webrtc = WebRtcChoice::from_policy(&p.browser.webrtc);

    container(
        column![
            section_title("配置概览", "保存后，使用此配置启动的应用会读取这些值"),
            container(
                column![
                    kv_row("地区", &crate::options::region_label(&p.locale.region)),
                    kv_row("Locale", &p.locale.locale_name),
                    kv_row("界面语言", &p.locale.ui_language),
                    kv_row("Windows 时区", &p.timezone.windows_id),
                    kv_row("Chrome / 检测代理", &p.browser.proxy.as_ref().map(|p| p.chrome_server()).unwrap_or_else(|| "无 Profile 代理".into())),
                    text("代理仅用于 Chrome 与出口查询；其他应用网络保持原有行为。")
                        .size(TEXT_CAPTION).color(MUTED).font(font::ui_font()),
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
                    dns::overview(&p.dns),
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
                        .size(TEXT_CAPTION)
                        .color(FAINT)
                        .font(font::ui_font()),
                    text("MAC 支持 Win32 和本机网卡 CIM 查询（Get-NetAdapter、Get-CimInstance）；远端查询保持远端值。")
                        .size(TEXT_CAPTION)
                        .color(FAINT)
                        .font(font::ui_font()),
                ]
                .spacing(7)
                .padding(10),
            )
            .width(Fill)
            .style(inner_card_style),

            button(
                text("删除环境配置")
                    .size(TEXT_CAPTION)
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

fn profile_editor(app: &EnvBoxApp) -> (Element<'_, Message>, Element<'_, Message>) {
    let region_opts = app.combo_options(crate::message::ComboField::Region);
    let locale_opts = app.combo_options(crate::message::ComboField::Locale);
    let ui_opts = app.combo_options(crate::message::ComboField::Ui);
    let tz_opts = app.combo_options(crate::message::ComboField::Timezone);
    let tz_iana_opts = app.combo_options(crate::message::ComboField::TimezoneIana);

    let basic = form_section(
        "基本信息",
        form_field(
            "配置名称",
            text_input("例如 US 开发环境", &app.profile_draft.name)
                .on_input(Message::ProfileName)
                .size(TEXT_BODY)
                .padding(Padding::from([8, 10]))
                .style(input_style)
                .font(font::ui_font()),
        ),
    );
    let mut language = column![
        form_field(
            "地区",
            searchable_select(
                crate::message::ComboField::Region,
                &app.profile_draft.region,
                "选择地区",
                app.open_combo == Some(crate::message::ComboField::Region),
                &app.combo_query,
                &region_opts,
                Message::ComboQuery,
                Message::ComboPick,
            ),
        ),
        form_field(
            "区域格式（Locale）",
            searchable_select(
                crate::message::ComboField::Locale,
                &app.profile_draft.locale,
                "选择区域格式",
                app.open_combo == Some(crate::message::ComboField::Locale),
                &app.combo_query,
                &locale_opts,
                Message::ComboQuery,
                Message::ComboPick,
            ),
        ),
        form_field(
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
        form_field(
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
    .spacing(14);
    if app.profile_advanced {
        language = language.push(form_field(
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
        ));
    }
    let advanced_toggle = button(
        row![
            icon(
                if app.profile_advanced {
                    Icon::ChevronDown
                } else {
                    Icon::MoreHorizontal
                },
                MUTED,
                14.0
            ),
            column![
                text("高级环境设置")
                    .size(TEXT_BODY)
                    .color(INK_2)
                    .font(font::name_font()),
                text("身份、IANA 时区、DNS、WebRTC 与变量")
                    .size(TEXT_CAPTION)
                    .color(MUTED)
                    .font(font::ui_font()),
            ]
            .spacing(4),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding(Padding::from([10, 12]))
    .style(secondary_btn)
    .on_press(Message::ProfileAdvancedToggle);
    let mut content = column![
        basic,
        form_section("语言与时区", language),
        timezone::view(app),
        advanced_toggle,
    ]
    .spacing(16);
    if app.profile_advanced {
        content = content.push(form_section("身份信息", column![
                row![
                    text("留空时跟随宿主").size(TEXT_CAPTION).color(MUTED).font(font::ui_font()),
                    tooltip(icon(Icon::Info, MUTED, 12.0),
                        container(column![
                            text("提供 Win32 主机名 / 用户名、网卡信息和 MachineGuid 注册表读取视图。真实账户、网卡和系统安装标识保持原值；CPU、GPU、磁盘身份暂不覆盖。").size(TEXT_CAPTION).font(font::ui_font()),
                            text("MAC 支持本机 Win32_NetworkAdapter、Win32_NetworkAdapterConfiguration 和 MSFT_NetAdapter 的 CIM 读取；其他身份字段的 WMI/CIM、GetUserNameEx、Native Registry API 和设备 IOCTL 暂不覆盖。").size(TEXT_CAPTION).font(font::ui_font()),
                        ].spacing(8)).width(320).padding(12),
                        tooltip::Position::Left,
                    ).style(inner_card_style),
                ].spacing(8).align_y(Alignment::Center),
                form_field(
                    "主机名",
                    text_input("1–15 位 ASCII 主机名", &app.profile_draft.identity.computer_name)
                        .on_input(Message::ProfileComputerName)
                        .size(TEXT_BODY)
                        .padding(Padding::from([8, 10]))
                        .style(input_style)
                        .font(font::ui_font()),
                ),
                form_field(
                    "用户名",
                    text_input("字母、数字、点、下划线或连字符", &app.profile_draft.identity.user_name)
                        .on_input(Message::ProfileUserName)
                        .size(TEXT_BODY)
                        .padding(Padding::from([8, 10]))
                        .style(input_style)
                        .font(font::ui_font()),
                ),
                form_field(
                    "MAC",
                    row![
                        text_input("02:AA:BB:CC:DD:EE", &app.profile_draft.identity.mac_address)
                            .on_input(Message::ProfileMacAddress)
                        .size(TEXT_BODY)
                            .padding(Padding::from([8, 10]))
                            .style(input_style)
                            .font(font::ui_font())
                            .width(Fill),
                        button(text("生成").size(TEXT_BODY).font(font::ui_font()))
                            .padding(Padding::from([8, 10]))
                            .style(secondary_btn)
                            .on_press(Message::ProfileMacAddressGenerate),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ),
                form_field(
                    "MachineGuid",
                    row![
                        text_input("Windows 安装标识 UUID", &app.profile_draft.identity.machine_guid)
                            .on_input(Message::ProfileMachineGuid)
                        .size(TEXT_BODY)
                            .padding(Padding::from([8, 10]))
                            .style(input_style)
                            .font(font::ui_font())
                            .width(Fill),
                        button(text("生成").size(TEXT_BODY).font(font::ui_font()))
                            .padding(Padding::from([8, 10]))
                            .style(secondary_btn)
                            .on_press(Message::ProfileMachineGuidGenerate),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ),

        ].spacing(14)));
        content = content.push(form_section(
            "DNS 与网络保护",
            column![
                form_field(
                    "DNS 模式",
                    pick_list(
                        DnsChoice::ALL,
                        Some(app.profile_draft.dns_mode),
                        Message::ProfileDnsMode
                    )
                    .style(pick_style)
                    .menu_style(pick_menu)
                    .padding(Padding::from([8, 10]))
                    .text_size(TEXT_BODY)
                    .font(font::ui_font())
                    .width(Fill),
                ),
                dns::view(
                    &app.profile_draft.dns_editor,
                    app.profile_draft.dns_mode.to_mode()
                ),
                iced::widget::horizontal_rule(1),
                form_field(
                    "WebRTC",
                    pick_list(
                        WebRtcChoice::ALL,
                        Some(app.profile_draft.webrtc),
                        Message::ProfileWebRtc
                    )
                    .style(pick_style)
                    .menu_style(pick_menu)
                    .padding(Padding::from([8, 10]))
                    .text_size(TEXT_BODY)
                    .font(font::ui_font())
                    .width(Fill),
                ),
                text("宿主模式保留浏览器原有行为；严格模式同时启用运行时网络保护。")
                    .size(TEXT_CAPTION)
                    .color(MUTED)
                    .font(font::ui_font()),
            ]
            .spacing(14),
        ));
        content = content.push(form_section(
            "环境变量",
            form_field(
                "变量列表",
                text_input("KEY=VALUE;KEY2=VALUE2", &app.profile_draft.env)
                    .on_input(Message::ProfileEnv)
                    .size(TEXT_BODY)
                    .padding(Padding::from([8, 10]))
                    .style(input_style)
                    .font(font::ui_font()),
            ),
        ));
    }
    let save = button(
        row![
            icon(Icon::Check, INK, 14.0),
            text("保存").size(TEXT_BODY).font(font::name_font())
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 14]))
    .width(Fill)
    .style(primary_btn)
    .on_press(Message::ProfileSave);
    let cancel = button(text("取消").size(TEXT_BODY).font(font::ui_font()))
        .padding(Padding::from([8, 12]))
        .style(secondary_btn)
        .on_press(Message::ProfileEditCancel);
    let mut actions = row![save, cancel].spacing(8).align_y(Alignment::Center);
    if app.profile_draft.id.is_some() {
        actions = actions.push(
            button(text("删除").size(TEXT_BODY).font(font::ui_font()))
                .padding(Padding::from([8, 10]))
                .style(danger_btn)
                .on_press(Message::ProfileDelete),
        );
    }
    (content.into(), actions.into())
}
fn dns_mode_label(mode: &envbox_core::DnsMode) -> &'static str {
    match mode {
        envbox_core::DnsMode::Host => "宿主",
        envbox_core::DnsMode::VirtualView => "虚拟视图",
    }
}
