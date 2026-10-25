//! IP-derived timezone preview; filling changes only the unsaved draft.
use crate::app::proxy_editor::ProxyChoice;
use crate::app::EnvBoxApp;
use crate::font;
use crate::message::Message;
use crate::theme::*;
use crate::widgets::{form_field, form_section, kv_row, primary_btn, secondary_btn};
use iced::widget::{button, column, pick_list, row, text, text_input};
use iced::{Alignment, Element, Fill, Padding};

pub(super) fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let state = &app.timezone_lookup;
    let query = button(
        text(if state.pending {
            "查询中…"
        } else {
            "查询时区"
        })
        .size(TEXT_BODY)
        .font(font::ui_font()),
    )
    .padding(Padding::from([8, 10]))
    .style(secondary_btn)
    .on_press_maybe((!state.pending).then_some(Message::TimezoneQuery));
    let proxy = &app.profile_draft.proxy;
    let mut body = column![form_field(
        "Chrome 代理",
        pick_list(
            ProxyChoice::ALL,
            Some(proxy.choice),
            Message::ProfileProxyChoice
        )
        .font(font::ui_font())
        .text_size(TEXT_BODY)
        .style(pick_style)
        .menu_style(pick_menu)
        .padding(Padding::from([8, 10]))
        .width(Fill),
    ),]
    .spacing(14);
    if proxy.choice != ProxyChoice::None {
        body = body.push(
            row![
                form_field(
                    "服务器",
                    text_input("IPv4 / IPv6 / 域名", &proxy.host)
                        .on_input(Message::ProfileProxyHost)
                        .font(font::ui_font())
                        .size(TEXT_BODY)
                        .padding(Padding::from([8, 10]))
                        .style(input_style)
                        .width(Fill),
                ),
                iced::widget::container(form_field(
                    "端口",
                    text_input("端口", &proxy.port)
                        .on_input(Message::ProfileProxyPort)
                        .font(font::ui_font())
                        .size(TEXT_BODY)
                        .padding(Padding::from([8, 10]))
                        .style(input_style),
                ))
                .width(85),
            ]
            .spacing(8),
        );
    }
    body = body.push(
        text("用于 Chrome 与出口查询，不支持认证；其他应用和 Windows 代理保持原样。")
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
    );
    body = body.push(
        row![
            text("根据出口查询时区")
                .size(TEXT_BODY)
                .font(font::name_font())
                .width(Fill),
            query,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );
    body = body.push(
        text(proxy.source_label())
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
    );
    body = body.push(
        text("未配置代理时查询 Aura 当前网络；浏览器扩展、代理绕过和站点分流可能产生不同出口。")
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
    );
    if let Some(error) = &state.error {
        body = body.push(
            text(error)
                .size(TEXT_CAPTION)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        );
    }
    if state.pending {
        body = body.push(
            button(text("取消查询").size(TEXT_BODY).font(font::ui_font()))
                .padding(Padding::from([8, 10]))
                .style(secondary_btn)
                .on_press(Message::TimezoneDismiss),
        );
    }
    if let Some(result) = &state.result {
        let fill = button(text("填入时区").size(TEXT_BODY).font(font::ui_font()))
            .padding(Padding::from([8, 10]))
            .style(primary_btn)
            .on_press_maybe(result.can_fill.then_some(Message::TimezoneFill));
        body = body.push(iced::widget::horizontal_rule(1));
        body = body.push(
            column![
                kv_row("查询来源", &result.source),
                kv_row("查询时间", &result.queried_at),
                kv_row("出口 IP", &result.ip.to_string()),
                kv_row(
                    "地区 / 城市",
                    if result.location.is_empty() {
                        "服务未提供"
                    } else {
                        &result.location
                    }
                ),
                kv_row(
                    "IANA 时区",
                    if result.iana.is_empty() {
                        "服务未提供"
                    } else {
                        &result.iana
                    }
                ),
                kv_row(
                    "Windows 时区",
                    result.windows.as_deref().unwrap_or("无法自动填入")
                ),
                text(&result.status)
                    .size(TEXT_CAPTION)
                    .color(MUTED)
                    .font(font::ui_font()),
                row![
                    fill,
                    button(text("关闭预览").size(TEXT_BODY).font(font::ui_font()))
                        .padding(Padding::from([8, 10]))
                        .style(secondary_btn)
                        .on_press(Message::TimezoneDismiss)
                ]
                .spacing(8),
                text("填入后点击“保存”，新的实例才会使用该时区。")
                    .size(TEXT_CAPTION)
                    .color(MUTED)
                    .font(font::ui_font()),
            ]
            .spacing(12),
        );
    }
    form_section("代理与出口查询", body)
}
