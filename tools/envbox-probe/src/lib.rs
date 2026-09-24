//! Probe gathers the Host environment view EnvBox later virtualizes.
//! Public surface is snapshot collection + stable text rendering (Probe seam).

use std::fmt::Write as _;

pub mod host;

/// Stable section headers used as independent expected values in tests.
pub const SECTION_GEO: &str = "GEO";
pub const SECTION_LOCALE: &str = "LOCALE";
pub const SECTION_LANGUAGE: &str = "LANGUAGE";
pub const SECTION_TIMEZONE: &str = "TIMEZONE";
pub const SECTION_DNS: &str = "DNS";
pub const SECTION_ENV: &str = "ENV";
pub const SECTION_REGISTRY: &str = "REGISTRY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSnapshot {
    pub sections: Vec<Section>,
}

impl HostSnapshot {
    /// Render the stable Probe text format: `=== TITLE ===` then `Name:\nValue` pairs.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            let _ = writeln!(out, "=== {} ===", section.title);
            for field in &section.fields {
                let _ = writeln!(out, "{}:", field.name);
                let _ = writeln!(out, "{}", field.value);
                let _ = writeln!(out);
            }
        }
        out
    }
}

pub fn collect_host_snapshot() -> HostSnapshot {
    host::collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_uses_stable_section_headers() {
        let snapshot = HostSnapshot {
            sections: vec![Section {
                title: SECTION_GEO.to_string(),
                fields: vec![Field {
                    name: "GetUserDefaultGeoName".into(),
                    value: "US".into(),
                }],
            }],
        };
        let text = snapshot.render();
        assert!(text.contains("=== GEO ==="), "missing GEO header in:\n{text}");
        assert!(text.contains("GetUserDefaultGeoName:"));
    }

    #[test]
    fn host_snapshot_contains_all_ticket01_sections() {
        let snapshot = collect_host_snapshot();
        let titles: Vec<_> = snapshot
            .sections
            .iter()
            .map(|s| s.title.as_str())
            .collect();
        for expected in [
            SECTION_GEO,
            SECTION_LOCALE,
            SECTION_LANGUAGE,
            SECTION_TIMEZONE,
            SECTION_DNS,
            SECTION_REGISTRY,
            SECTION_ENV,
        ] {
            assert!(
                titles.contains(&expected),
                "missing section {expected}; got {titles:?}"
            );
        }
    }
}
