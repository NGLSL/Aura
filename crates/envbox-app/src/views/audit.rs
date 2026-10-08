//! Full-page audit log.

use iced::widget::{button, column, container, pick_list, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::{self, pick_menu, pick_style, BORDER, FAINT, INK, INK_2, MUTED, SUCCESS_BG, SUCCESS_TEXT};
use crate::widgets::{badge, empty_state, primary_btn, secondary_btn};

/// `2026-09-25T15:08:59.877Z` → `2026-09-25 15:08:59`.
fn format_ts(ts: &str) -> String {
    let s = ts.trim();
    if s.len() >= 19 {
        let head = &s[..19];
        format!("{} {}", &head[..10], &head[11..])
    } else {
        s.to_string()
    }
}

pub fn view_full(app: &EnvBoxApp) -> Element<'_, Message> {
    use crate::message::Nav;

    let rows_all = app.filtered_audit_events();
    let shown = rows_all.len();
    let total = app.audit_total;
    let mut soft_opts = vec!["全部软件".to_string()];
    soft_opts.extend(app.audit_software_options());
    let filter_sel = if app.audit_filter.trim().is_empty() {
        "全部软件".to_string()
    } else {
        app.audit_filter.clone()
    };

    let header = row![
        column![
            text("安全审计").size(24).color(INK).font(font::name_font()),
            text(format!(
                "审计记录 · 显示 {} / 共捕获 {} 条 · 点击刷新更新记录",
                shown, total
            ))
            .size(12)
            .color(MUTED)
            .font(font::ui_font()),
        ]
        .spacing(4)
        .width(Fill),
        row![
            pick_list(soft_opts, Some(filter_sel), |s: String| {
                Message::AuditFilter(if s == "全部软件" { String::new() } else { s })
            })
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([7, 10]))
            .font(font::ui_font())
            .width(Length::Fixed(180.0)),
            button(
                row![
                    icon(Icon::Folder, INK_2, 12.0),
                    text("打开日志目录").size(12).color(INK_2).font(font::ui_font())
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([7, 14]))
            .style(secondary_btn)
            .on_press(Message::OpenAuditDir),
            button(
                row![
                    icon(Icon::Document, INK_2, 12.0),
                    text("刷新日志").size(12).color(INK_2).font(font::ui_font())
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([7, 14]))
            .style(secondary_btn)
            .on_press(Message::AuditRefresh),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .align_y(Alignment::Center);

    let content: Element<_> = if shown == 0 {
        let guide_btn = button(
            row![
                icon(Icon::Play, INK, 11.0),
                text("前往应用配置开启审计")
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
            Icon::Document,
            if total == 0 {
                "暂无环境调用拦截记录"
            } else {
                "当前软件筛选下暂无记录"
            },
            "在应用配置中勾选开启「审计环境读取 (Audit Mode)」，运行该应用时所有对 Windows 时区、区域、语言及 DNS 的敏感读取都将在此毫秒级记录。",
            Some(guide_btn.into()),
        )
    } else {
        let table_header = container(
            row![
                text("时间 (UTC)")
                    .size(11)
                    .color(FAINT)
                    .font(font::ui_font())
                    .width(Length::Fixed(140.0)),
                text("软件")
                    .size(11)
                    .color(FAINT)
                    .font(font::ui_font())
                    .width(Length::Fixed(120.0)),
                text("PID")
                    .size(11)
                    .color(FAINT)
                    .font(font::ui_font())
                    .width(Length::Fixed(64.0)),
                text("系统 API 拦截调用与虚拟化返回结果")
                    .size(11)
                    .color(FAINT)
                    .font(font::ui_font())
                    .width(Fill),
                text("判定状态")
                    .size(11)
                    .color(FAINT)
                    .font(font::ui_font())
                    .width(Length::Fixed(90.0)),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([6, 14]))
        .width(Fill);

        let rows = rows_all.iter().fold(column![].spacing(6), |col, ev| {
            let summary = ev.summary.clone().unwrap_or_default();
            let bg = if ev.virtualized { SUCCESS_BG } else { BORDER };
            let fg = if ev.virtualized { SUCCESS_TEXT } else { MUTED };
            let label = if ev.virtualized {
                "VIRTUALIZED"
            } else {
                "HOST PASS"
            };
            let call_str = if summary.is_empty() {
                format!("{}()", ev.api)
            } else {
                format!("{}() → {}", ev.api, summary)
            };
            let call_str = if ev.n > 1 {
                format!("{call_str} ×{}", ev.n)
            } else {
                call_str
            };
            let soft = app.audit_software_label(ev);

            let card = container(
                row![
                    text(format_ts(&ev.ts_utc))
                        .size(11)
                        .color(MUTED)
                        .font(font::ui_font())
                        .width(Length::Fixed(140.0)),
                    text(soft)
                        .size(11)
                        .color(theme::ACCENT_TEXT)
                        .font(font::ui_font())
                        .width(Length::Fixed(120.0)),
                    text(format!("#{}", ev.pid))
                        .size(11)
                        .color(MUTED)
                        .font(font::ui_font())
                        .width(Length::Fixed(64.0)),
                    text(call_str)
                        .size(12)
                        .color(INK_2)
                        .font(font::name_font())
                        .width(Fill),
                    badge(label, bg, fg),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([9, 14]))
            .width(Fill)
            .style(theme::inner_card_style);

            col.push(card)
        });

        column![
            table_header,
            scrollable(rows)
                .height(Fill)
                .style(theme::dark_scrollable)
        ]
        .spacing(6)
        .into()
    };

    column![header, content]
        .spacing(16)
        .width(Fill)
        .height(Fill)
        .into()
}
