//! Window preferences, local data, diagnostics and application updates.

use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::{self, INK, INK_2, MUTED, TEXT_BODY, TEXT_CAPTION, TEXT_PAGE_TITLE};
use crate::widgets::{form_section, primary_btn, secondary_btn};

fn setting_row<'a>(
    label: &'static str,
    detail: String,
    action: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        column![
            text(label)
                .size(TEXT_BODY)
                .color(INK_2)
                .font(font::ui_font()),
            text(detail)
                .size(TEXT_CAPTION)
                .color(MUTED)
                .font(font::ui_font())
                .width(Fill),
        ]
        .spacing(4)
        .width(Fill),
        action.into(),
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

fn action_button(label: &str, message: Message) -> Element<'static, Message> {
    button(
        text(label.to_string())
            .size(TEXT_BODY)
            .font(font::ui_font()),
    )
    .padding(Padding::from([8, 12]))
    .style(secondary_btn)
    .on_press(message)
    .into()
}

fn info_row(label: &str, value: String) -> Element<'static, Message> {
    row![
        text(label.to_string())
            .size(TEXT_BODY)
            .color(MUTED)
            .font(font::ui_font())
            .width(Length::Fixed(100.0)),
        text(value)
            .size(TEXT_BODY)
            .color(INK_2)
            .font(font::ui_font())
            .width(Fill),
    ]
    .spacing(14)
    .into()
}

fn update_hint(app: &EnvBoxApp) -> String {
    match &app.update_status {
        None => "手动检查 GitHub 上的最新稳定版本".into(),
        Some(Err(error)) => format!("检查失败：{error}"),
        Some(Ok(crate::updater::CheckResult::UpToDate)) => "当前已是最新版本".into(),
        Some(Ok(crate::updater::CheckResult::Available {
            tag,
            installer: Some(_),
        })) if app.update_installer_path.is_some() => {
            format!("发现 {tag}，安装包已完成校验")
        }
        Some(Ok(crate::updater::CheckResult::Available {
            tag,
            installer: Some(_),
        })) => format!("发现 {tag}，可下载并安装"),
        Some(Ok(crate::updater::CheckResult::Available {
            tag,
            installer: None,
        })) => format!("发现 {tag}，可从发布页面下载安装包"),
    }
}

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let header = column![
        text("设置")
            .size(TEXT_PAGE_TITLE)
            .color(INK)
            .font(font::name_font()),
        text("窗口行为、数据目录与版本更新")
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(6);

    let general = form_section(
        "常规",
        setting_row(
            "关闭主窗口",
            format!(
                "当前：{}。适用于关闭按钮和 Alt+F4。",
                app.close_behavior.label()
            ),
            action_button("重新选择", Message::WindowClosePreferenceReset),
        ),
    );

    let audit_summary = if app.audit_loading {
        "正在读取审计记录…".into()
    } else {
        format!(
            "共 {} 条记录，可在审计页查看最近 {} 条。",
            app.audit_total,
            app.audit_events.len()
        )
    };
    let data = form_section(
        "数据与诊断",
        column![
            setting_row(
                "配置目录",
                app.store.root().display().to_string(),
                action_button("打开目录", Message::OpenConfigDir),
            ),
            info_row(
                "已保存配置",
                format!(
                    "{} 个应用 · {} 个环境配置",
                    app.applications.len(),
                    app.profiles.len()
                ),
            ),
            setting_row(
                "审计日志",
                audit_summary,
                action_button("打开日志", Message::OpenAuditDir),
            ),
            setting_row(
                "环境诊断",
                "在独立窗口中查看语言、时区和 DNS 的读取结果。".into(),
                button(
                    row![
                        icon(Icon::Play, INK, 12.0),
                        text("启动探针").size(TEXT_BODY).font(font::ui_font()),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .padding(Padding::from([8, 12]))
                .style(primary_btn)
                .on_press(Message::RunProbe),
            ),
        ]
        .spacing(18),
    );

    let update_label = if app.update_checking {
        "处理中…"
    } else if app.update_installer_path.is_some() {
        "安装更新"
    } else if app.update_asset.is_some() {
        "下载并安装"
    } else {
        "检查更新"
    };
    let update_action = if app.update_checking {
        None
    } else if app.update_asset.is_some() {
        Some(Message::InstallUpdate)
    } else {
        Some(Message::CheckUpdate)
    };
    let about = form_section(
        "关于 Aura",
        column![
            info_row("应用版本", crate::version::product_label()),
            info_row(
                "核心引擎",
                format!("{} {}", crate::version::ENGINE_NAME, crate::version::PRODUCT_VERSION),
            ),
            setting_row(
                "版本更新",
                update_hint(app),
                button(text(update_label).size(TEXT_BODY).font(font::ui_font()))
                    .padding(Padding::from([8, 12]))
                    .style(secondary_btn)
                    .on_press_maybe(update_action),
            ),
            row![
                action_button("发布页面", Message::OpenReleases),
                action_button("源代码", Message::OpenRepository),
            ]
            .spacing(8),
            text("Windows 原生运行 · Microsoft Detours。环境信息视图作用于启动的进程树，目标仍可使用本机文件、网络和当前账户权限。")
                .size(TEXT_CAPTION)
                .color(MUTED)
                .font(font::ui_font())
                .width(Fill),
        ]
        .spacing(16),
    );

    let content = column![header, general, data, about]
        .spacing(18)
        .width(Fill)
        .max_width(820);
    scrollable(
        container(content)
            .width(Fill)
            .center_x(Fill)
            .padding(Padding::from([22, 24])),
    )
    .height(Fill)
    .style(theme::dark_scrollable)
    .into()
}
