//! Profiles center list.

use iced::widget::{button, column, row, scrollable, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{Message, WebRtcChoice};
use crate::theme::{self, ACCENT_BG, ACCENT_LINE, ACCENT_TEXT, INK, MUTED};
use crate::widgets::{badge, primary_btn};

pub fn view_center(app: &EnvBoxApp) -> Element<'_, Message> {
    let header = column![
        column![
            text("环境配置")
                .size(26)
                .color(INK)
                .font(font::name_font())
                .width(Fill),
            button(
                row![
                    text("+").size(15).color(INK).font(font::name_font()),
                    text("新建环境配置")
                        .size(13)
                        .color(INK)
                        .font(font::ui_font()),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([8, 16]))
            .style(primary_btn)
            .on_press(Message::ProfileNew),
        ]
        .spacing(12),
        text("地区、语言、时区、DNS 与身份信息")
            .size(13)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(6);

    let content: Element<_> = if app.profiles.is_empty() {
        let new_btn = button(
            row![
                icon(Icon::Play, INK, 11.0),
                text("新建第一个环境配置")
                    .size(13)
                    .color(INK)
                    .font(font::name_font()),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([9, 20]))
        .style(primary_btn)
        .on_press(Message::ProfileNew);

        crate::widgets::empty_state(
            Icon::Globe,
            "暂无环境配置",
            "创建一组应用可以复用的地区、语言、时区、网络和环境变量设置。",
            Some(new_btn.into()),
        )
    } else {
        let list = app.profiles.iter().fold(column![].spacing(10), |col, p| {
            let selected = app.profile_draft.id == Some(p.id);
            let dns_label = match &p.dns.mode {
                envbox_core::DnsMode::Host => "DNS: 宿主",
                envbox_core::DnsMode::VirtualView => "DNS: 虚拟视图",
            };
            let dns_bg = match &p.dns.mode {
                envbox_core::DnsMode::Host => theme::BORDER,
                envbox_core::DnsMode::VirtualView => ACCENT_BG,
            };
            let dns_fg = match &p.dns.mode {
                envbox_core::DnsMode::Host => MUTED,
                envbox_core::DnsMode::VirtualView => ACCENT_TEXT,
            };
            let webrtc_choice = WebRtcChoice::from_policy(&p.browser.webrtc);
            let webrtc_active = p.browser.webrtc != envbox_core::WebRtcPolicy::Host;

            let badges = row![
                badge(&p.locale.region, ACCENT_BG, ACCENT_TEXT),
                badge(dns_label, dns_bg, dns_fg),
                badge(
                    webrtc_choice.short_label(),
                    if webrtc_active {
                        ACCENT_BG
                    } else {
                        theme::BORDER
                    },
                    if webrtc_active { ACCENT_TEXT } else { MUTED },
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center);

            let mut metadata = row![].spacing(6).align_y(Alignment::Center);
            if !p.environment.is_empty() {
                metadata = metadata.push(badge(
                    &format!("Env: {} 项", p.environment.len()),
                    theme::BORDER,
                    MUTED,
                ));
            }

            let identity_count = [
                &p.identity.computer_name,
                &p.identity.user_name,
                &p.identity.mac_address,
                &p.identity.machine_guid,
            ]
            .into_iter()
            .filter(|value| value.is_some())
            .count();
            if identity_count != 0 {
                metadata = metadata.push(badge(
                    &format!("身份: {identity_count} 项"),
                    ACCENT_BG,
                    ACCENT_TEXT,
                ));
            }

            let mut card = column![
                row![
                    icon(
                        Icon::Globe,
                        if selected { ACCENT_LINE } else { MUTED },
                        16.0
                    ),
                    column![
                        text(&p.name)
                            .size(15)
                            .color(if selected { ACCENT_TEXT } else { INK })
                            .font(font::name_font()),
                        text(format!(
                            "{} · {} · {}",
                            p.locale.region, p.locale.locale_name, p.timezone.windows_id
                        ))
                        .size(12)
                        .color(MUTED)
                        .font(font::ui_font()),
                    ]
                    .spacing(2)
                    .width(Fill),
                ]
                .spacing(14)
                .align_y(Alignment::Center),
                badges,
            ]
            .spacing(10);
            if !p.environment.is_empty() || identity_count != 0 {
                card = card.push(metadata);
            }
            col.push(
                button(card)
                    .padding(14)
                    .width(Fill)
                    .style(move |_t, _s| theme::list_card_style(selected))
                    .on_press(Message::ProfileSelect(p.id)),
            )
        });

        scrollable(list)
            .height(Fill)
            .style(theme::dark_scrollable)
            .into()
    };

    column![header, content]
        .spacing(14)
        .width(Fill)
        .height(Fill)
        .into()
}
