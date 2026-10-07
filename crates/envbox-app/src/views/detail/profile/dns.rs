use iced::widget::{
    button, checkbox, column, container, pick_list, row, text, text_input, tooltip,
};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::dns_editor::{DnsEditor, TransportChoice};
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::*;
use crate::widgets::{field_label, secondary_btn};

pub(super) fn view(editor: &DnsEditor, mode: envbox_core::DnsMode) -> Element<'_, Message> {
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
        .to_profile(mode)
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
