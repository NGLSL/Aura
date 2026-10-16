use iced::widget::{
    button, checkbox, column, container, pick_list, row, text, text_input, tooltip, Space,
};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::dns_editor::{DnsEditor, TransportChoice};
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::*;
use crate::widgets::{field_label, secondary_btn};

pub(super) fn view(editor: &DnsEditor, mode: envbox_core::DnsMode) -> Element<'_, Message> {
    let explanation = column![
        text("严格模式：受支持的 DNS API 解析失败时，禁止回退宿主 DNS。")
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
    let mut content = column![
        row![
            text("DNS 上游顺序")
                .size(13)
                .color(INK)
                .font(font::name_font())
                .width(Fill),
            help,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        checkbox("禁止回退宿主 DNS（严格模式）", editor.strict)
            .on_toggle(Message::ProfileDnsStrict)
            .text_size(12)
            .font(font::ui_font())
            .width(Fill),
        text(if mode == envbox_core::DnsMode::Host {
            "当前跟随宿主 DNS；以下上游在虚拟模式下使用。"
        } else {
            "按从上到下的顺序尝试，失败后使用下一项。"
        })
        .size(12)
        .color(MUTED)
        .font(font::ui_font()),
    ]
    .spacing(10)
    .width(Fill);
    if editor
        .to_profile(mode)
        .ok()
        .is_some_and(|dns| dns.validate_runtime_support().is_err())
    {
        content = content.push(
            text("配置可以保存；当前 Runtime 不支持此协议、端口或非严格模式组合，启动会拒绝。")
                .size(12)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        );
    }
    if editor
        .upstreams
        .iter()
        .any(envbox_core::DnsUpstream::is_plaintext)
    {
        content = content.push(
            text("含明文上游：按顺序尝试到 UDP / TCP 时，将发送明文查询。")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }
    if editor.upstreams.is_empty() {
        content = content.push(
            text("尚未添加上游")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }
    for (index, upstream) in editor.upstreams.iter().enumerate() {
        let mut entry = column![upstream_summary(index, upstream)]
            .spacing(8)
            .width(Fill);
        entry = entry.push(
            row![
                button(text("上移").size(12).font(font::ui_font()))
                    .padding(Padding::from([5, 10]))
                    .style(secondary_btn)
                    .on_press_maybe((index > 0).then_some(Message::ProfileDnsMove(index, true))),
                button(text("下移").size(12).font(font::ui_font()))
                    .padding(Padding::from([5, 10]))
                    .style(secondary_btn)
                    .on_press_maybe(
                        (index + 1 < editor.upstreams.len())
                            .then_some(Message::ProfileDnsMove(index, false))
                    ),
                Space::new(Fill, 1),
                button(text("移除").size(12).font(font::ui_font()))
                    .padding(Padding::from([5, 10]))
                    .style(secondary_btn)
                    .on_press(Message::ProfileDnsRemove(index)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
        content = content.push(
            container(entry)
                .width(Fill)
                .padding(12)
                .style(inner_card_style),
        );
    }
    let mut draft = column![
        text("添加 DNS 上游")
            .size(13)
            .color(INK_2)
            .font(font::name_font()),
        field_label("协议"),
        pick_list(
            TransportChoice::ALL,
            Some(editor.draft.transport),
            Message::ProfileDnsTransport,
        )
        .style(pick_style)
        .menu_style(pick_menu)
        .padding(Padding::from([6, 8]))
        .font(font::ui_font())
        .text_size(12)
        .width(Fill),
    ]
    .spacing(8)
    .width(Fill);
    if editor.draft.transport == TransportChoice::Doh {
        draft = draft.push(field_label("DoH URL"));
        draft = draft.push(
            text_input("https://resolver.example/dns-query", &editor.draft.url)
                .on_input(Message::ProfileDnsUrl)
                .style(input_style)
                .padding(8)
                .font(font::ui_font())
                .size(12),
        );
        draft = draft.push(field_label("连接 IP · 可选"));
        draft = draft.push(
            text_input("IP 地址，多个用逗号分隔", &editor.draft.bootstrap)
                .on_input(Message::ProfileDnsBootstrap)
                .style(input_style)
                .padding(8)
                .font(font::ui_font())
                .size(12),
        );
        draft = draft.push(
            text("留空时由 Profile 中可直接连接的上游解析服务域名；无可用上游则失败，不使用宿主 DNS。手填 IP 将覆盖自动解析。")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        );
        draft = draft.push(field_label("TLS 证书验证"));
        draft = draft.push(
            pick_list(
                envbox_core::DnsTlsRevocation::ALL,
                Some(editor.draft.tls_revocation),
                Message::ProfileDnsTlsRevocation,
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([6, 8]))
            .font(font::ui_font())
            .text_size(12)
            .width(Fill),
        );
    } else {
        draft = draft.push(
            row![
                column![
                    field_label("IP 地址"),
                    text_input("例如 1.1.1.1", &editor.draft.address)
                        .on_input(Message::ProfileDnsAddress)
                        .style(input_style)
                        .padding(8)
                        .font(font::ui_font())
                        .size(12),
                ]
                .spacing(8)
                .width(Fill),
                column![
                    field_label("端口"),
                    text_input("端口", &editor.draft.port)
                        .on_input(Message::ProfileDnsPort)
                        .style(input_style)
                        .padding(8)
                        .font(font::ui_font())
                        .size(12),
                ]
                .spacing(8)
                .width(76),
            ]
            .spacing(8),
        );
        if editor.draft.transport == TransportChoice::Dot {
            draft = draft.push(field_label("TLS 证书身份"));
            draft = draft.push(
                text_input("例如 cloudflare-dns.com", &editor.draft.server_name)
                    .on_input(Message::ProfileDnsServerName)
                    .style(input_style)
                    .padding(8)
                    .font(font::ui_font())
                    .size(12),
            );
        }
    }
    if let Some(error) = editor.draft_error() {
        draft = draft.push(text(error).size(12).color(DANGER).font(font::ui_font()));
    }
    draft = draft.push(
        button(text("添加上游").size(12).font(font::ui_font()))
            .padding(Padding::from([6, 12]))
            .style(secondary_btn)
            .on_press(Message::ProfileDnsAdd),
    );
    content
        .push(container(draft).padding(Padding::from([6, 0])))
        .into()
}

pub(super) fn overview(profile: &envbox_core::DnsProfile) -> Element<'static, Message> {
    let mut content = column![row![
        field_label("DNS"),
        text(super::dns_mode_label(&profile.mode))
            .size(12)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(8)
    .align_y(Alignment::Center),]
    .spacing(8)
    .width(Fill);
    if profile.mode == envbox_core::DnsMode::Host {
        return content
            .push(
                text("跟随系统 DNS")
                    .size(12)
                    .color(INK_2)
                    .font(font::ui_font()),
            )
            .into();
    }
    content = content.push(
        text(if profile.strict {
            "严格模式 · 禁止回退宿主 DNS"
        } else {
            "非严格模式"
        })
        .size(12)
        .color(MUTED)
        .font(font::ui_font()),
    );
    let upstreams = profile.effective_upstreams();
    if upstreams.is_empty() {
        content = content.push(
            text("尚未添加上游")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }
    for (index, upstream) in upstreams.iter().enumerate() {
        content = content.push(
            container(upstream_summary(index, upstream))
                .padding(Padding::from([4, 0]))
                .width(Fill),
        );
    }
    content.into()
}

fn upstream_summary(
    index: usize,
    upstream: &envbox_core::DnsUpstream,
) -> Element<'static, Message> {
    let (transport, endpoint, details) = match upstream {
        envbox_core::DnsUpstream::Udp { address, port } => (
            "UDP · 明文",
            std::net::SocketAddr::new(*address, *port).to_string(),
            None,
        ),
        envbox_core::DnsUpstream::Tcp { address, port } => (
            "TCP · 明文",
            std::net::SocketAddr::new(*address, *port).to_string(),
            None,
        ),
        envbox_core::DnsUpstream::Dot {
            address,
            port,
            server_name,
        } => (
            "DoT · 加密",
            std::net::SocketAddr::new(*address, *port).to_string(),
            Some(format!("证书身份：{server_name}")),
        ),
        envbox_core::DnsUpstream::Doh {
            url,
            bootstrap_ips,
            tls_revocation,
        } => (
            "DoH · 加密",
            url.clone(),
            Some(format!(
                "{}\nTLS：{tls_revocation}",
                if bootstrap_ips.is_empty() {
                    "连接 IP：自动；服务域名由 Profile 中可直接连接的上游解析，不使用宿主 DNS。"
                        .to_string()
                } else {
                    format!(
                        "手动连接 IP：\n{}",
                        bootstrap_ips
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                }
            )),
        ),
    };
    let mut heading = row![text(format!("{} · {transport}", index + 1))
        .size(12)
        .color(INK_2)
        .font(font::name_font())
        .width(Fill),]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(details) = details {
        heading = heading.push(
            tooltip(
                text("连接与证书")
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
                container(
                    text(details)
                        .size(12)
                        .font(font::ui_font())
                        .wrapping(iced::widget::text::Wrapping::Glyph),
                )
                .width(300)
                .padding(12)
                .style(inner_card_style),
                tooltip::Position::Left,
            )
            .gap(8)
            .style(inner_card_style),
        );
    }
    column![
        heading,
        text(endpoint)
            .size(12)
            .color(INK_2)
            .font(font::ui_font())
            .wrapping(iced::widget::text::Wrapping::Glyph)
            .width(Fill),
    ]
    .spacing(4)
    .width(Fill)
    .into()
}
