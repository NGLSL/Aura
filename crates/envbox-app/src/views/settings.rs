//! Settings and environment overview.

use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::{self, ACCENT_BG, ACCENT_LINE, ACCENT_TEXT, BORDER, FAINT, INK, INK_2, MUTED, SUCCESS_BG, SUCCESS_TEXT};
use crate::widgets::{badge, primary_btn, secondary_btn};

fn section_card<'a>(
    ic: Icon,
    title: &'static str,
    subtitle: &'static str,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let header = row![
        icon(ic, ACCENT_LINE, 15.0),
        column![
            text(title).size(14).color(INK).font(font::name_font()),
            text(subtitle).size(11).color(MUTED).font(font::ui_font()),
        ]
        .spacing(2),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    container(
        column![header, content]
            .spacing(14)
            .padding(18),
    )
    .width(Fill)
    .style(theme::panel_card_style)
    .into()
}

fn info_row<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    row![
        text(label)
            .size(12)
            .color(MUTED)
            .font(font::ui_font())
            .width(Length::Fixed(130.0)),
        text(value)
            .size(12)
            .color(INK_2)
            .font(font::ui_font())
            .width(Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

fn info_row_with_action<'a>(
    label: &'static str,
    value: String,
    btn_text: &'static str,
    action: Message,
) -> Element<'a, Message> {
    row![
        text(label)
            .size(12)
            .color(MUTED)
            .font(font::ui_font())
            .width(Length::Fixed(130.0)),
        text(value)
            .size(12)
            .color(INK_2)
            .font(font::ui_font())
            .width(Fill),
        button(
            row![
                icon(Icon::Folder, INK_2, 11.0),
                text(btn_text).size(11).color(INK_2).font(font::ui_font()),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([5, 10]))
        .style(secondary_btn)
        .on_press(action),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let header = column![
        text("设置与系统概览").size(24).color(INK).font(font::name_font()),
        text("Aura 1.0.0 · 核心引擎 EnvBox 进程级环境虚拟化").size(12).color(MUTED).font(font::ui_font()),
    ]
    .spacing(4);

    let engine_content = column![
        info_row("宿主操作系统", "Windows x86_64".into()),
        info_row("控制台程序", "Aura 1.0.0 (Native GUI)".into()),
        info_row("虚拟化引擎", "EnvBox Core · 进程级环境隔离".into()),
        info_row("底层注入框架", "Microsoft Detours 4.0.1 x64".into()),
        info_row("运行透明度", "目标进程直接运行在宿主 OS · 共享文件系统、网络与 GPU".into()),
    ]
    .spacing(8);

    let engine_card = section_card(
        Icon::Monitor,
        "系统架构与运行时核心",
        "原生 Windows 执行模型，不修改宿主全局状态",
        engine_content.into(),
    );

    let storage_content = column![
        info_row_with_action(
            "配置根目录",
            app.store.root().display().to_string(),
            "打开目录",
            Message::OpenConfigDir,
        ),
        info_row(
            "应用清单",
            format!("applications.toml (共 {} 个配置项)", app.applications.len()),
        ),
        info_row(
            "环境 Profile 清单",
            format!("profiles.toml (共 {} 个配置项)", app.profiles.len()),
        ),
        info_row_with_action(
            "审计事件记录",
            format!("audit/ (当前已载入 {} 条事件)", app.audit_events.len()),
            "打开日志",
            Message::OpenAuditDir,
        ),
    ]
    .spacing(8);

    let storage_card = section_card(
        Icon::Folder,
        "配置数据与存储目录",
        "本地 TOML 持久化存储与实时审计日志",
        storage_content.into(),
    );

    let invariants_content = column![
        row![
            container(badge("进程级隔离", ACCENT_BG, ACCENT_TEXT)).width(Length::Fixed(86.0)),
            text("仅作用于 RuntimeInstance 及其派生子进程树，零宿主污染。")
                .size(12)
                .color(INK_2)
                .font(font::ui_font()),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        row![
            container(badge("宿主透明", SUCCESS_BG, SUCCESS_TEXT)).width(Length::Fixed(86.0)),
            text("绝不修改 Windows 全局注册表、系统语言、区域及全局系统时区。")
                .size(12)
                .color(INK_2)
                .font(font::ui_font()),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        row![
            container(badge("安全降级", SUCCESS_BG, SUCCESS_TEXT)).width(Length::Fixed(86.0)),
            text("Hook 异常或未处理 API 时优先直通 Windows 原生调用，禁止静默失败。")
                .size(12)
                .color(INK_2)
                .font(font::ui_font()),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        row![
            container(badge("全系统访问", BORDER, MUTED)).width(Length::Fixed(86.0)),
            text("直接访问宿主文件系统、GPU、网络及当前用户权限，不是安全沙箱。")
                .size(12)
                .color(INK_2)
                .font(font::ui_font()),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    ]
    .spacing(10);

    let invariants_card = section_card(
        Icon::Check,
        "核心设计原则与安全保证",
        "遵循 EnvBox 架构设计准则，保证系统稳定可靠",
        invariants_content.into(),
    );

    let probe_content = column![
        text("环境探针 (envbox-probe) 会输出当前进程感知到的 ANSI/OEM 代码页、Locale 标识符、时区标准名称/夏令时规则以及 DNS 解析链，用于验收虚拟化精准度。")
            .size(12)
            .color(INK_2)
            .font(font::ui_font()),
        row![
            button(
                row![
                    icon(Icon::Play, INK, 11.0),
                    text("启动环境探针 (envbox-probe)").size(12).color(INK).font(font::name_font()),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([8, 16]))
            .style(primary_btn)
            .on_press(Message::RunProbe),
            text("将在独立控制台窗口中运行并输出诊断基准")
                .size(11)
                .color(FAINT)
                .font(font::ui_font()),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    ]
    .spacing(12);

    let probe_card = section_card(
        Icon::Terminal,
        "环境基准诊断",
        "运行 envbox-probe 实时比对宿主环境与虚拟环境",
        probe_content.into(),
    );

    let page_content = column![
        header,
        engine_card,
        storage_card,
        invariants_card,
        probe_card,
    ]
    .spacing(16)
    .max_width(820);

    scrollable(
        container(page_content)
            .width(Fill)
            .center_x(Fill)
            .padding(Padding::from([10, 24])),
    )
    .height(Fill)
    .style(theme::dark_scrollable)
    .into()
}

