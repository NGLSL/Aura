//! CJK UI font selection (ported from Veya).
//!
//! On Windows, `msyh.ttc` family is `微软雅黑` / `Microsoft YaHei UI`.
//! System UI fonts like `Segoe UI` have no CJK glyphs — picking those first
//! yields tofu boxes. Whitelist CJK families and force sans/serif fallback.

use std::sync::OnceLock;

use iced::font::{Family, Font};

struct UiFont {
    font: Font,
}

static UI: OnceLock<UiFont> = OnceLock::new();

const CJK_FAMILIES: &[&str] = &[
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "微软雅黑",
    "DengXian",
    "等线",
    "SimHei",
    "黑体",
    "Noto Sans SC",
    "Source Han Sans SC",
    "Source Han Sans CN",
    "Noto Sans CJK SC",
    "PingFang SC",
];

const CJK_FILES: &[&str] = &[
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\msyhbd.ttc",
    r"C:\Windows\Fonts\Deng.ttf",
    r"C:\Windows\Fonts\simhei.ttf",
];

/// Install/select the default UI font (CJK). First call does the work.
pub fn install() -> Font {
    init().font
}

pub fn ui_font() -> Font {
    init().font
}

/// Titles / names: same face as body. Do not change weight — YaHei bold
/// often fails to match and falls back to tofu.
pub fn name_font() -> Font {
    ui_font()
}

/// Monospace for ASCII ids/paths only — never Chinese (Consolas has no CJK).
#[allow(dead_code)]
pub fn mono_font() -> Font {
    Font {
        family: Family::Name("Consolas"),
        ..ui_font()
    }
}

fn init() -> &'static UiFont {
    UI.get_or_init(|| {
        use iced_graphics::text::font_system;
        let mut fs = font_system().write().expect("iced font system");
        let db = fs.raw().db_mut();
        let mut family = CJK_FAMILIES.iter().copied().find(|name| {
            db.faces().any(|face| {
                face.families
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case(name))
            })
        });
        let mut fallback_files = 0;
        if family.is_none() {
            for path in CJK_FILES {
                if db.load_font_file(path).is_ok() {
                    fallback_files += 1;
                    family = CJK_FAMILIES.iter().copied().find(|name| {
                        db.faces().any(|face| {
                            face.families
                                .iter()
                                .any(|(n, _)| n.eq_ignore_ascii_case(name))
                        })
                    });
                    if family.is_some() {
                        break;
                    }
                }
            }
        }
        let family = family.unwrap_or("Microsoft YaHei UI");

        db.set_sans_serif_family(family);
        db.set_serif_family(family);
        db.set_monospace_family("Consolas");

        let font = Font {
            family: Family::Name(family),
            ..Font::DEFAULT
        };
        eprintln!("[envbox-font] family={family} fallback_files={fallback_files}");
        UiFont { font }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_font_is_a_cjk_family_not_latin() {
        let font = install();
        let Family::Name(name) = font.family else {
            panic!("expected Family::Name, got {:?}", font.family);
        };
        assert!(!name.is_empty());
        for latin in ["Segoe UI", "Fira Sans", "Fira Mono", "Consolas", "Arial"] {
            assert_ne!(name, latin, "must not pick Latin-only family");
        }
    }

    #[test]
    fn name_font_matches_ui_face_exactly() {
        assert_eq!(name_font().weight, ui_font().weight);
        assert_eq!(name_font().family, ui_font().family);
    }
}
