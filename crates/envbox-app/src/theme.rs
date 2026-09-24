//! Palette and widget styles for the dark three-column shell.
//! Visual tokens live here; views must not hard-code raw hex.

#![allow(dead_code)]

use iced::widget::{button, container, overlay, pick_list, text, text_input};
use iced::{Background, Border, Color, Gradient, Radians, Theme};

// Clean dark navy slate theme matching Figure 2
pub const WINDOW: Color = Color::from_rgb(0.043, 0.059, 0.086);     // #0B0F16
pub const SIDEBAR: Color = Color::from_rgb(0.035, 0.047, 0.071);    // #090C12
pub const SIDEBAR_TOP: Color = Color::from_rgb(0.043, 0.059, 0.086);// #0B0F16
pub const PANEL: Color = Color::from_rgb(0.051, 0.071, 0.102);      // #0D121A
pub const PANEL_RIGHT: Color = Color::from_rgb(0.043, 0.059, 0.086);// #0B0F16
pub const INPUT: Color = Color::from_rgb(0.035, 0.047, 0.071);      // #090C12
pub const CARD: Color = Color::from_rgb(0.075, 0.106, 0.157);       // #131B28
pub const CARD_IDLE: Color = Color::from_rgb(0.059, 0.082, 0.122);  // #0F151F
pub const CARD_INNER: Color = Color::from_rgb(0.043, 0.059, 0.086); // #0B0F16
pub const BORDER: Color = Color::from_rgb(0.106, 0.141, 0.196);     // #1B2432
pub const BORDER_SOFT: Color = Color::from_rgb(0.141, 0.188, 0.259);// #243042
pub const BORDER_BTN: Color = Color::from_rgb(0.153, 0.208, 0.290); // #27354A

// Accents
pub const ACCENT: Color = Color::from_rgb(0.114, 0.408, 0.949);     // #1D68F2
pub const ACCENT_HOVER: Color = Color::from_rgb(0.145, 0.450, 0.980);// #2573FA
pub const ACCENT_SOFT: Color = Color::from_rgb(0.102, 0.278, 0.706); // #1A47B4
pub const ACCENT_LINE: Color = Color::from_rgb(0.231, 0.510, 0.965); // #3B82F6
pub const ACCENT_TEXT: Color = Color::from_rgb(0.376, 0.647, 0.980); // #60A5FA
pub const ACCENT_BG: Color = Color::from_rgb(0.082, 0.153, 0.271);   // #152745

// Logos / App marks
pub const LOGO_A: Color = Color::from_rgb(0.220, 0.741, 0.973);     // #38BDF8
pub const LOGO_B: Color = Color::from_rgb(0.114, 0.408, 0.949);     // #1D68F2

// Typography
pub const INK: Color = Color::from_rgb(0.973, 0.980, 0.988);        // #F8FAFC
pub const INK_2: Color = Color::from_rgb(0.886, 0.910, 0.941);      // #E2E8F0
pub const MUTED: Color = Color::from_rgb(0.550, 0.612, 0.698);      // #8C9CB2
pub const FAINT: Color = Color::from_rgb(0.392, 0.455, 0.545);      // #64748B

// Status colors
pub const SUCCESS: Color = Color::from_rgb(0.133, 0.773, 0.369);     // #22C55E
pub const SUCCESS_TEXT: Color = Color::from_rgb(0.290, 0.871, 0.502);// #4ADE80
pub const SUCCESS_BG: Color = Color::from_rgb(0.059, 0.200, 0.118); // #0F331E
pub const DANGER: Color = Color::from_rgb(0.937, 0.267, 0.267);      // #EF4444
pub const DANGER_TEXT: Color = Color::from_rgb(0.996, 0.647, 0.647); // #FEA5A5
pub const DANGER_BG: Color = Color::from_rgb(0.220, 0.080, 0.080);

/// Linear gradient background.
pub fn gradient(start: Color, end: Color) -> Background {
    Gradient::Linear(
        iced::gradient::Linear::new(Radians(1.2))
            .add_stop(0.0, start)
            .add_stop(1.0, end),
    )
    .into()
}

pub fn panel_style(color: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        text_color: Some(INK),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub fn sidebar_style(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SIDEBAR)),
        text_color: Some(INK),
        border: Border {
            color: BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub fn card_style(selected: bool) -> container::Style {
    container::Style {
        background: Some(Background::Color(if selected { CARD } else { CARD_IDLE })),
        text_color: Some(INK),
        border: Border {
            color: if selected { ACCENT_LINE } else { BORDER },
            width: if selected { 1.5 } else { 1.0 },
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

pub fn panel_card_style(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(CARD_IDLE)),
        text_color: Some(INK),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

pub fn inner_card_style(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(CARD_INNER)),
        text_color: Some(INK),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

pub fn badge_style(bg: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(bg)),
        text_color: Some(INK),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

pub fn icon_tile_style(bg: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(bg)),
        text_color: Some(INK),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..Default::default()
    }
}

pub fn input_style(_t: &Theme, _s: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(INPUT),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        icon: MUTED,
        placeholder: FAINT,
        value: INK_2,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

pub fn borderless_input(_t: &Theme, _s: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        icon: MUTED,
        placeholder: FAINT,
        value: INK_2,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

pub fn search_box_container(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(INPUT)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Dark pick_list body (closed control).
pub fn pick_style(_t: &Theme, status: pick_list::Status) -> pick_list::Style {
    let active = matches!(
        status,
        pick_list::Status::Hovered | pick_list::Status::Opened { .. }
    );
    pick_list::Style {
        text_color: if active { INK } else { INK_2 },
        placeholder_color: FAINT,
        handle_color: if active { ACCENT_TEXT } else { MUTED },
        background: Background::Color(if active {
            CARD
        } else {
            INPUT
        }),
        border: Border {
            color: if matches!(status, pick_list::Status::Opened { .. }) {
                ACCENT_LINE
            } else {
                BORDER
            },
            width: 1.0,
            radius: 8.0.into(),
        },
    }
}

/// Dark pick_list popup menu.
pub fn pick_menu(_t: &Theme) -> overlay::menu::Style {
    overlay::menu::Style {
        background: Background::Color(CARD),
        border: Border {
            color: BORDER_SOFT,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: INK_2,
        selected_text_color: INK,
        selected_background: Background::Color(ACCENT_BG),
    }
}

/// Title-bar window control buttons (— □ ×).
pub fn win_button(_t: &Theme, status: button::Status) -> button::Style {
    let (bg, fg) = match status {
        button::Status::Hovered => (CARD_IDLE, INK),
        button::Status::Pressed => (BORDER, INK),
        _ => (Color::TRANSPARENT, MUTED),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

pub fn win_close_button(_t: &Theme, status: button::Status) -> button::Style {
    let (bg, fg) = match status {
        button::Status::Hovered => (DANGER, INK),
        button::Status::Pressed => (Color::from_rgb(0.75, 0.15, 0.15), INK),
        _ => (Color::TRANSPARENT, MUTED),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

/// Custom title bar strip.
pub fn titlebar_style(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SIDEBAR)),
        text_color: Some(MUTED),
        border: Border {
            color: BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// Outer undecorated window shell.
pub fn shell_style(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(WINDOW)),
        text_color: Some(INK),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

pub fn primary_btn(_t: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => ACCENT_HOVER,
        button::Status::Pressed => ACCENT_SOFT,
        _ => ACCENT,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        text_color: INK,
        ..Default::default()
    }
}

pub fn secondary_btn(_t: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.118, 0.165, 0.235),
        button::Status::Pressed => Color::from_rgb(0.067, 0.098, 0.145),
        _ => Color::from_rgb(0.082, 0.118, 0.173),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: BORDER_BTN,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: INK_2,
        ..Default::default()
    }
}

pub fn danger_btn(_t: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.260, 0.100, 0.100),
        button::Status::Pressed => Color::from_rgb(0.180, 0.060, 0.060),
        _ => DANGER_BG,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: DANGER,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: DANGER_TEXT,
        ..Default::default()
    }
}

pub fn nav_btn_style(selected: bool) -> button::Style {
    if selected {
        button::Style {
            background: Some(Background::Color(Color::from_rgb(0.08, 0.15, 0.26))),
            border: Border {
                color: Color::from_rgb(0.18, 0.38, 0.70),
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: INK,
            ..Default::default()
        }
    } else {
        button::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 8.0.into(),
            },
            text_color: MUTED,
            ..Default::default()
        }
    }
}

pub fn tab_btn_style(selected: bool) -> button::Style {
    if selected {
        button::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            text_color: ACCENT_LINE,
            ..Default::default()
        }
    } else {
        button::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            text_color: MUTED,
            ..Default::default()
        }
    }
}

pub fn list_card_style(selected: bool) -> button::Style {
    if selected {
        button::Style {
            background: Some(Background::Color(CARD)),
            border: Border {
                color: ACCENT_LINE,
                width: 1.5,
                radius: 12.0.into(),
            },
            text_color: INK,
            ..Default::default()
        }
    } else {
        button::Style {
            background: Some(Background::Color(CARD_IDLE)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 12.0.into(),
            },
            text_color: INK,
            ..Default::default()
        }
    }
}
pub fn dark_scrollable(_t: &Theme, status: iced::widget::scrollable::Status) -> iced::widget::scrollable::Style {
    let thumb_alpha = match status {
        iced::widget::scrollable::Status::Hovered { .. } => 0.35,
        iced::widget::scrollable::Status::Dragged { .. } => 0.55,
        _ => 0.18,
    };
    let rail = iced::widget::scrollable::Rail {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        border: Border::default(),
        scroller: iced::widget::scrollable::Scroller {
            color: Color::from_rgba(1.0, 1.0, 1.0, thumb_alpha),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 3.0.into(),
            },
        },
    };
    iced::widget::scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
    }
}
/// Title text with product font.
#[allow(dead_code)]
pub fn title(s: &str, size: u16) -> text::Text<'static, Theme, iced::Renderer> {
    text(s.to_string())
        .size(size)
        .color(INK)
        .font(crate::font::name_font())
}

#[allow(dead_code)]
pub fn body(s: &str, size: u16) -> text::Text<'static, Theme, iced::Renderer> {
    text(s.to_string())
        .size(size)
        .color(INK_2)
        .font(crate::font::ui_font())
}

#[allow(dead_code)]
pub fn muted(s: &str, size: u16) -> text::Text<'static, Theme, iced::Renderer> {
    text(s.to_string())
        .size(size)
        .color(MUTED)
        .font(crate::font::ui_font())
}

#[allow(dead_code)]
pub fn faint(s: &str, size: u16) -> text::Text<'static, Theme, iced::Renderer> {
    text(s.to_string())
        .size(size)
        .color(FAINT)
        .font(crate::font::ui_font())
}

/// Color pair per app name for fallback icons.
pub fn app_gradient(name: &str) -> (Color, Color) {
    let mut h: u32 = 0;
    for b in name.bytes() {
        h = h.wrapping_mul(16777619) ^ (b as u32);
    }
    let palette: &[(Color, Color)] = &[
        (
            Color::from_rgb(0.95, 0.45, 0.20),
            Color::from_rgb(0.85, 0.25, 0.15),
        ), // orange
        (
            Color::from_rgb(0.20, 0.65, 0.95),
            Color::from_rgb(0.12, 0.35, 0.88),
        ), // cyan-blue
        (
            Color::from_rgb(0.55, 0.30, 0.95),
            Color::from_rgb(0.35, 0.15, 0.65),
        ), // purple
        (
            Color::from_rgb(0.15, 0.75, 0.40),
            Color::from_rgb(0.08, 0.50, 0.30),
        ), // green
    ];
    palette[(h as usize) % palette.len()]
}
