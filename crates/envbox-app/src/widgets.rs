//! Shared UI primitives. No domain logic.

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Border, Color, Element, Fill, Length, Padding};

use crate::font;
use crate::icons::{icon, Icon};
use crate::message::Message;
use crate::theme::{
    self, input_style, ACCENT, ACCENT_BG, ACCENT_LINE, BORDER, BORDER_SOFT, INK, INK_2, MUTED,
};

pub fn field_label(s: &str) -> Element<'static, Message> {
    text(s.to_string())
        .size(12)
        .color(INK_2)
        .font(font::name_font())
        .into()
}

pub fn form_row<'a>(
    label: &'a str,
    field: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        text(label.to_string())
            .size(12)
            .color(MUTED)
            .font(font::ui_font())
            .width(Length::Fixed(88.0)),
        field.into(),
    ]
    .spacing(8)
    .align_y(Alignment::Start)
    .into()
}

/// Combobox: closed = one pick_list-style row; open = search + option list
/// under the same header (search lives in the dropdown, not a second permanent field).
pub fn searchable_select(
    field: crate::message::ComboField,
    value: &str,
    placeholder: &'static str,
    open: bool,
    query: &str,
    options: &[String],
    _on_query: fn(String) -> Message,
    on_pick: fn(crate::message::ComboField, String) -> Message,
) -> Element<'static, Message> {
    use iced::widget::{scrollable, text_input};
    use crate::icons::Icon;
    use crate::message::ComboField;
    use crate::options::{option_label, OptionKind};

    let kind = match field {
        ComboField::Region => OptionKind::Region,
        ComboField::Locale | ComboField::Ui => OptionKind::Locale,
        ComboField::Timezone | ComboField::TimezoneIana => OptionKind::Timezone,
    };

    let value = value.to_string();
    let query = query.to_string();
    let options = options.to_vec();

    let display = if value.trim().is_empty() {
        placeholder.to_string()
    } else {
        option_label(kind, &value)
    };
    let display_color = if value.trim().is_empty() {
        MUTED
    } else {
        INK_2
    };

    let header = button(
        row![
            text(display)
                .size(12)
                .color(display_color)
                .font(font::ui_font()),
            iced::widget::Space::new(Fill, 1.0),
            icon(Icon::ChevronDown, if open { ACCENT_LINE } else { MUTED }, 11.0),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding(Padding::from([5, 8]))
    .style(theme::combo_btn)
    .on_press(Message::ComboToggle(field));

    if !open {
        return header.into();
    }

    let filtered = crate::options::filter_options(&options, &query, kind);
    let mut list = column![].spacing(2);
    if filtered.is_empty() {
        list = list.push(
            text("无匹配项")
                .size(11)
                .color(MUTED)
                .font(font::ui_font())
                .width(Fill),
        );
    } else {
        for opt in filtered.iter().take(60) {
            let raw = (*opt).to_string();
            let label = option_label(kind, &raw);
            let selected = raw == value;
            list = list.push(
                button(
                    row![
                        text(label)
                            .size(12)
                            .color(if selected { ACCENT_LINE } else { INK_2 })
                            .font(font::ui_font()),
                        iced::widget::Space::new(Fill, 1.0),
                        icon(Icon::Check, if selected { ACCENT_LINE } else { Color::TRANSPARENT }, 11.0),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .width(Fill)
                .padding(Padding::from([6, 8]))
                .style(theme::combo_option)
                .on_press(on_pick(field, raw)),
            );
        }
    }

    let panel = container(
        column![
            text_input("输入以搜索…", &query)
                .on_input(Message::ComboQuery)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
                .width(Fill),
            scrollable(list)
                .height(Length::Fixed(160.0))
                .style(theme::dark_scrollable),
        ]
        .spacing(6)
        .padding(Padding::from([8, 6])),
    )
    .width(Fill)
    .style(theme::combo_panel);

    crate::combo_overlay::DropdownOverlay::new(header, Some(panel)).into()
}

pub fn kv_row(label: &str, value: &str) -> Element<'static, Message> {
    row![
        text(label.to_string())
            .size(12)
            .color(MUTED)
            .font(font::ui_font())
            .width(Length::Fixed(90.0)),
        text(value.to_string())
            .size(12)
            .color(INK_2)
            .font(font::ui_font()),
    ]
    .spacing(12)
    .into()
}

pub fn status_dot(color: Color) -> Element<'static, Message> {
    // Pure geometric circular status indicator (exact pixel-centered dot).
    container(iced::widget::Space::new(Length::Fixed(7.0), Length::Fixed(7.0)))
        .width(Length::Fixed(7.0))
        .height(Length::Fixed(7.0))
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(color)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 3.5.into(),
            },
            ..Default::default()
        })
        .into()
}

pub fn toggle(on: bool, msg: fn(bool) -> Message) -> Element<'static, Message> {
    // Modern pill track + circular knob switch matching Figure 2.
    let knob = container(text(" ").size(8))
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(if on {
                INK
            } else {
                Color::from_rgb(0.65, 0.72, 0.80)
            })),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 7.0.into(),
            },
            ..Default::default()
        })
        .center_x(14.0)
        .center_y(14.0);

    button(
        container(knob)
            .width(Fill)
            .height(Fill)
            .padding(Padding::from([3.0, 3.0]))
            .align_x(if on { Alignment::End } else { Alignment::Start })
            .align_y(Alignment::Center),
    )
    .width(Length::Fixed(38.0))
    .height(Length::Fixed(22.0))
    .padding(0)
    .style(move |_t, _s| button::Style {
        background: Some(iced::Background::Color(if on {
            ACCENT
        } else {
            Color::from_rgb(0.141, 0.188, 0.259)
        })),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 11.0.into(),
        },
        text_color: INK,
        ..Default::default()
    })
    .on_press(msg(!on))
    .into()
}

pub fn badge(label: &str, bg: Color, fg: Color) -> Element<'static, Message> {
    container(
        text(label.to_string())
            .size(11)
            .color(fg)
            .font(font::ui_font()),
    )
    .padding(Padding::from([2, 8]))
    .style(move |_| theme::badge_style(bg))
    .into()
}

pub fn app_initials(name: &str) -> String {
    name.chars().take(2).collect::<String>().to_uppercase()
}

pub fn short_id(id: uuid::Uuid) -> String {
    let s = id.to_string();
    format!("{}…{}", &s[..4], &s[s.len() - 2..])
}

/// Brand or monogram icon badge for applications, matching Figure 2.
pub fn app_icon_badge(name: &str, size: f32) -> Element<'static, Message> {
    let lower = name.trim().to_lowercase();
    // Keep the brand mark visually comparable to extracted icons. The SVG
    // viewboxes already include a little breathing room, so 58% made the
    // glyph look noticeably undersized inside a 40px tile.
    let inner_size = size * 0.68;

    if lower.contains("claude") {
        container(icon(
            Icon::Claude,
            Color::from_rgb(0.95, 0.45, 0.20),
            inner_size,
        ))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.12, 0.08, 0.06))),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..Default::default()
        })
        .center_x(size)
        .center_y(size)
        .into()
    } else if lower.contains("codex") || lower.contains("chatgpt") || lower.contains("openai") {
        container(icon(Icon::OpenAi, INK, inner_size))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .style(|_| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.06, 0.09, 0.13))),
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..Default::default()
            })
            .center_x(size)
            .center_y(size)
            .into()
    } else if lower.contains("mimo") {
        container(icon(Icon::Mimo, INK, inner_size))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .style(|_| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.06, 0.08, 0.12))),
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..Default::default()
            })
            .center_x(size)
            .center_y(size)
            .into()
    } else if lower.contains("probe") {
        container(icon(
            Icon::Flask,
            Color::from_rgb(0.66, 0.33, 0.97),
            inner_size,
        ))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.10, 0.07, 0.17))),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..Default::default()
        })
        .center_x(size)
        .center_y(size)
        .into()
    } else {
        let (ga, gb) = theme::app_gradient(name);
        container(
            text(app_initials(name))
                .size((size * 0.38) as u16)
                .color(INK)
                .font(font::name_font()),
        )
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_| container::Style {
            background: Some(theme::gradient(ga, gb)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 10.0.into(),
            },
            ..Default::default()
        })
        .center_x(size)
        .center_y(size)
        .into()
    }
}

/// Underline tab with icon and bottom accent indicator line.
#[allow(dead_code)]
pub fn underline_tab_button<T: Copy + PartialEq + 'static>(
    label: &str,
    tab_icon: Option<Icon>,
    tab: T,
    current: T,
    on_select: impl Fn(T) -> Message + 'static,
) -> Element<'static, Message> {
    let selected = tab == current;
    let color = if selected { ACCENT_LINE } else { MUTED };

    let mut row_items = row![].spacing(6).align_y(Alignment::Center);
    if let Some(ic) = tab_icon {
        row_items = row_items.push(icon(ic, color, 14.0));
    }
    row_items = row_items.push(
        text(label.to_string())
            .size(12)
            .color(color)
            .font(font::ui_font()),
    );

    let indicator = container(text(" "))
        .height(Length::Fixed(2.0))
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(if selected {
                ACCENT_LINE
            } else {
                Color::TRANSPARENT
            })),
            ..Default::default()
        });

    let content = column![row_items, indicator].spacing(6);

    let btn = button(content)
        .padding(Padding::from([6, 12]))
        .style(move |_t, _s| button::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            text_color: color,
            ..Default::default()
        });

    if selected {
        btn.into()
    } else {
        btn.on_press(on_select(tab)).into()
    }
}

pub use theme::{danger_btn, primary_btn, secondary_btn};

/// Polished, centered empty state container with glowing icon, title, subtitle and optional action.
pub fn empty_state<'a, Message: 'static>(
    ic: Icon,
    title: &'static str,
    desc: &'static str,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let icon_circle = container(icon(ic, ACCENT_LINE, 28.0))
        .width(Length::Fixed(64.0))
        .height(Length::Fixed(64.0))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(ACCENT_BG)),
            border: Border {
                color: BORDER_SOFT,
                width: 1.0,
                radius: 32.0.into(),
            },
            ..Default::default()
        })
        .center_x(64.0)
        .center_y(64.0);

    let mut col = column![
        icon_circle,
        text(title)
            .size(17)
            .color(INK)
            .font(font::name_font()),
        text(desc)
            .size(13)
            .color(MUTED)
            .font(font::ui_font()),
    ]
    .spacing(12)
    .align_x(Alignment::Center);

    if let Some(act) = action {
        col = col.push(container(act).padding(Padding::from([6, 0])));
    }

    container(col)
        .padding(Padding::from([56, 24]))
        .width(Fill)
        .center_x(Fill)
        .center_y(Fill)
        .into()
}
