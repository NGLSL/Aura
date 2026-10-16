//! IP-derived timezone preview; filling changes only the unsaved draft.
use crate::app::proxy_editor::ProxyChoice;
use crate::app::EnvBoxApp;
use crate::font;
use crate::message::Message;
use crate::theme::*;
use crate::widgets::{field_label, kv_row, primary_btn, secondary_btn};
use iced::widget::{button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Fill, Padding};

pub(super) fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let state = &app.timezone_lookup;
    let query = button(
        text(if state.pending {
            "查询中…"
        } else {
            "查询时区"
        })
        .size(12)
        .font(font::ui_font()),
    )
    .padding(Padding::from([6, 10]))
    .style(secondary_btn)
    .on_press_maybe((!state.pending).then_some(Message::TimezoneQuery));
    let proxy = &app.profile_draft.proxy;
    let mut body = column![
        field_label("Chrome 与出口检测代理"),
        pick_list(
            ProxyChoice::ALL,
            Some(proxy.choice),
            Message::ProfileProxyChoice
        )
        .font(font::ui_font())
        .text_size(12)
        .style(pick_style)
        .menu_style(pick_menu)
        .padding(Padding::from([5, 8]))
        .width(Fill),
    ]
    .spacing(8);
    if proxy.choice != ProxyChoice::None {
        body = body.push(
            row![
                text_input("服务器 IPv4 / IPv6 / 域名", &proxy.host)
                    .on_input(Message::ProfileProxyHost)
                    .font(font::ui_font())
                    .size(12)
                    .padding(Padding::from([6, 8]))
                    .style(input_style)
                    .width(Fill),
                text_input("端口", &proxy.port)
                    .on_input(Message::ProfileProxyPort)
                    .font(font::ui_font())
                    .size(12)
                    .padding(Padding::from([6, 8]))
                    .style(input_style)
                    .width(85),
            ]
            .spacing(8),
        );
    }
    body = body.push(
        text("仅用于 Chrome 与本次出口查询；不支持认证，不改变 Windows 全局代理或其他应用网络。")
            .size(12)
            .color(MUTED)
            .font(font::ui_font()),
    );
    body = body.push(
        row![field_label("查询实际出口与时区"), query]
            .spacing(8)
            .align_y(Alignment::Center),
    );
    body = body.push(
        text(proxy.source_label())
            .size(12)
            .color(MUTED)
            .font(font::ui_font()),
    );
    body = body.push(text("无 Profile 代理时查询 Aura 当前网络；浏览器扩展、代理绕过及站点分流仍可能产生不同出口。")
        .size(12).color(MUTED).font(font::ui_font()));
    if let Some(error) = &state.error {
        body = body.push(
            text(error)
                .size(12)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        );
    }
    if state.pending {
        body = body.push(
            button(text("取消查询").size(12).font(font::ui_font()))
                .padding(Padding::from([5, 9]))
                .style(secondary_btn)
                .on_press(Message::TimezoneDismiss),
        );
    }
    if let Some(result) = &state.result {
        let fill = button(text("填入时区").size(12).font(font::ui_font()))
            .padding(Padding::from([6, 10]))
            .style(primary_btn)
            .on_press_maybe(result.can_fill.then_some(Message::TimezoneFill));
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
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
                row![
                    fill,
                    button(text("关闭预览").size(12).font(font::ui_font()))
                        .padding(Padding::from([6, 10]))
                        .style(secondary_btn)
                        .on_press(Message::TimezoneDismiss)
                ]
                .spacing(8),
                text("填入后点击“保存”，新的实例才会使用该时区。")
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
            ]
            .spacing(7),
        );
    }
    container(body.padding(10))
        .width(Fill)
        .style(inner_card_style)
        .into()
}
