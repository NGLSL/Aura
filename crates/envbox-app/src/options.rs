//! Curated Profile editor option lists (common values only).
//! Display: only 中国 / 美国 / timezone Chinese names. Stored values stay host tokens.

/// Region list is intentionally tiny (search still finds the current value).
pub const COMMON_REGIONS: &[&str] = &["CN", "US"];

/// Locale / UI language: keep the two common ones.
pub const COMMON_LOCALES: &[&str] = &["zh-CN", "en-US"];

/// Region display: 中国 / 美国 only. Other codes stay as codes (no long names).
pub fn region_label(code: &str) -> String {
    match code.trim().to_ascii_uppercase().as_str() {
        "CN" => "中国".into(),
        "US" => "美国".into(),
        _ => code.to_string(),
    }
}

/// Locale / UI language: show the token itself (no long language labels).
pub fn locale_label(locale: &str) -> String {
    locale.trim().to_string()
}

/// Windows / IANA timezone → short Chinese name. Never leak raw English ids.
/// Covers every pair in `envbox_storage::COMMON_TIMEZONE_PAIRS` + host leftovers.
pub fn timezone_label(tz: &str) -> String {
    let t = tz.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.contains('/') || t.eq_ignore_ascii_case("utc") {
        return iana_label(t);
    }
    windows_tz_label(t)
}

fn iana_label(t: &str) -> String {
    let key = t.trim();
    let mapped = match key {
        "Asia/Shanghai" | "Asia/Chongqing" => Some("上海"),
        "Asia/Urumqi" => Some("乌鲁木齐"),
        "Asia/Hong_Kong" => Some("香港"),
        "Asia/Taipei" => Some("台北"),
        "Asia/Tokyo" => Some("东京"),
        "Asia/Seoul" => Some("首尔"),
        "Asia/Singapore" => Some("新加坡"),
        "Asia/Jerusalem" => Some("耶路撒冷"),
        "Asia/Dubai" => Some("迪拜"),
        "Asia/Kolkata" | "Asia/Calcutta" => Some("加尔各答"),
        "Asia/Dhaka" => Some("达卡"),
        "Asia/Bangkok" => Some("曼谷"),
        "America/New_York" => Some("纽约"),
        "America/Chicago" => Some("芝加哥"),
        "America/Denver" => Some("丹佛"),
        "America/Los_Angeles" => Some("洛杉矶"),
        "America/Anchorage" => Some("安克雷奇"),
        "America/Halifax" => Some("哈利法克斯"),
        "America/Bogota" => Some("波哥大"),
        "America/Sao_Paulo" => Some("圣保罗"),
        "America/Santiago" => Some("圣地亚哥"),
        "America/Mexico_City" => Some("墨西哥城"),
        "Pacific/Honolulu" => Some("檀香山"),
        "Pacific/Auckland" => Some("奥克兰"),
        "Europe/London" => Some("伦敦"),
        "Europe/Berlin" => Some("柏林"),
        "Europe/Paris" => Some("巴黎"),
        "Europe/Budapest" => Some("布达佩斯"),
        "Europe/Warsaw" => Some("华沙"),
        "Europe/Kyiv" | "Europe/Kiev" => Some("基辅"),
        "Europe/Istanbul" => Some("伊斯坦布尔"),
        "Europe/Moscow" => Some("莫斯科"),
        "Australia/Sydney" => Some("悉尼"),
        "Australia/Perth" => Some("珀斯"),
        "Africa/Johannesburg" => Some("约翰内斯堡"),
        "Africa/Cairo" => Some("开罗"),
        "UTC" | "utc" => Some("UTC"),
        _ => None,
    };
    match mapped {
        Some(s) => s.to_string(),
        // Fallback: last path segment as city-ish token, never "America/New_York".
        None => key.rsplit('/').next().unwrap_or(key).replace('_', " "),
    }
}

fn windows_tz_label(t: &str) -> String {
    let lower = t.trim().to_ascii_lowercase();
    let label = match lower.as_str() {
        "china standard time" => "中国标准时间",
        "taipei standard time" => "台北时间",
        "hong kong standard time" => "香港时间",
        "tokyo standard time" => "东京时间",
        "korea standard time" => "首尔时间",
        "singapore standard time" => "新加坡时间",
        "pacific standard time" => "太平洋时间",
        "us pacific standard time" => "太平洋时间",
        "mountain standard time" => "山地时间",
        "us mountain standard time" => "山地时间",
        "central standard time" => "中部时间",
        "us central standard time" => "中部时间",
        "eastern standard time" => "东部时间",
        "us eastern standard time" => "东部时间",
        "alaskan standard time" => "阿拉斯加时间",
        "hawaiian standard time" => "夏威夷时间",
        "atlantic standard time" => "大西洋时间",
        "sa pacific standard time" => "南美太平洋时间",
        "e. south america standard time" => "南美东部时间",
        "e. south america daylight time" => "南美东部时间",
        "pacific sa standard time" => "南美西部时间",
        "mexico standard time" => "墨西哥时间",
        "central america standard time" => "中美洲时间",
        "gmt standard time" => "英国时间",
        "greenwich standard time" => "格林尼治时间",
        "w. europe standard time" => "西欧时间",
        "romance standard time" => "中欧西岸时间",
        "central europe standard time" => "中欧时间",
        "central european standard time" => "中欧标准时间",
        "fle standard time" => "东欧时间",
        "gtb standard time" => "东欧标准时间",
        "turkey standard time" => "土耳其时间",
        "russian standard time" => "俄罗斯时间",
        "israel standard time" => "以色列时间",
        "arabian standard time" => "阿拉伯时间",
        "arab standard time" => "阿拉伯时间",
        "india standard time" => "印度时间",
        "bangladesh standard time" => "孟加拉时间",
        "se asia standard time" => "东南亚时间",
        "myanmar standard time" => "缅甸时间",
        "aus eastern standard time" => "澳洲东部时间",
        "aus central standard time" => "澳洲中部时间",
        "w. australia standard time" => "澳洲西部时间",
        "new zealand standard time" => "新西兰时间",
        "south africa standard time" => "南非时间",
        "egypt standard time" => "埃及时间",
        "morocco standard time" => "摩洛哥时间",
        "west asia standard time" => "西亚时间",
        "central asia standard time" => "中亚时间",
        "n. central asia standard time" => "北中亚时间",
        "north asia standard time" => "北亚时间",
        "north asia east standard time" => "东北亚时间",
        "yakutsk standard time" => "雅库茨克时间",
        "vladivostok standard time" => "海参崴时间",
        "sakhalin standard time" => "库页岛时间",
        "kamchatka standard time" => "堪察加时间",
        "central pacific standard time" => "中太平洋时间",
        "west pacific standard time" => "西太平洋时间",
        "fiji standard time" => "斐济时间",
        "tonga standard time" => "汤加时间",
        "samoa standard time" => "萨摩亚时间",
        "aleutian standard time" => "阿留申时间",
        "utc" => "UTC",
        _ => "",
    };
    if !label.is_empty() {
        return label.to_string();
    }
    // Host leftovers: strip English timezone boilerplate, then localize region words.
    let mut s = t.trim().to_string();
    for noise in [
        " Standard Time",
        " Daylight Time",
        " Standard",
        " Daylight",
        " Time",
    ] {
        if let Some(pos) = s.to_ascii_lowercase().find(&noise.to_ascii_lowercase()) {
            s.replace_range(pos..pos + noise.len(), "");
        }
    }
    let s = s.trim();
    if s.is_empty() {
        return "UTC".to_string();
    }
    localize_tz_words(s)
}

/// Map leftover English timezone region words to Chinese so the list never looks half-English.
fn localize_tz_words(s: &str) -> String {
    const WORDS: &[(&str, &str)] = &[
        ("E. South America", "南美东部"),
        ("S. America", "南美"),
        ("South America", "南美"),
        ("North America", "北美"),
        ("Central America", "中美洲"),
        ("Central Europe", "中欧"),
        ("Eastern Europe", "东欧"),
        ("Western Europe", "西欧"),
        ("W. Europe", "西欧"),
        ("E. Europe", "东欧"),
        ("Northern Europe", "北欧"),
        ("Southern Europe", "南欧"),
        ("Europe", "欧洲"),
        ("US Pacific", "美国太平洋"),
        ("US Mountain", "美国山地"),
        ("US Central", "美国中部"),
        ("US Eastern", "美国东部"),
        ("Pacific SA", "南美西部"),
        ("SA Pacific", "南美太平洋"),
        ("North Asia East", "东北亚"),
        ("N. Central Asia", "北中亚"),
        ("North Asia", "北亚"),
        ("Central Asia", "中亚"),
        ("West Asia", "西亚"),
        ("SE Asia", "东南亚"),
        ("South Asia", "南亚"),
        ("East Asia", "东亚"),
        ("Asia", "亚洲"),
        ("Pacific", "太平洋"),
        ("Atlantic", "大西洋"),
        ("Indian", "印度洋"),
        ("Africa", "非洲"),
        ("Australia", "澳洲"),
        ("AUS Eastern", "澳洲东部"),
        ("AUS Central", "澳洲中部"),
        ("W. Australia", "澳洲西部"),
        ("New Zealand", "新西兰"),
        ("Mountain", "山地"),
        ("Central", "中部"),
        ("Eastern", "东部"),
        ("Western", "西部"),
        ("Alaskan", "阿拉斯加"),
        ("Hawaiian", "夏威夷"),
        ("Aleutian", "阿留申"),
        ("Mexico", "墨西哥"),
        ("Canada", "加拿大"),
        ("Greenland", "格陵兰"),
        ("GMT", "格林尼治"),
        ("UTC", "UTC"),
        ("Coordinated Universal", "协调世界"),
    ];

    let mut out = s.to_string();
    let mut changed = true;
    while changed {
        changed = false;
        for (en, zh) in WORDS {
            let lower = out.to_ascii_lowercase();
            if let Some(pos) = lower.find(&en.to_ascii_lowercase()) {
                out.replace_range(pos..pos + en.len(), zh);
                changed = true;
            }
        }
    }
    let out = out.trim();
    if out.is_empty() {
        "时间".to_string()
    } else if out.contains("时间") {
        out.to_string()
    } else {
        format!("{out}时间")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Region,
    Locale,
    Timezone,
}

/// Generic display label for combo options.
pub fn option_label(kind: OptionKind, value: &str) -> String {
    match kind {
        OptionKind::Region => region_label(value),
        OptionKind::Locale => locale_label(value),
        OptionKind::Timezone => timezone_label(value),
    }
}

pub fn common_regions() -> Vec<String> {
    COMMON_REGIONS.iter().map(|s| (*s).to_string()).collect()
}

pub fn common_locales() -> Vec<String> {
    COMMON_LOCALES.iter().map(|s| (*s).to_string()).collect()
}

/// Filter by value or display label (case-insensitive).
pub fn filter_options<'a>(options: &'a [String], query: &str, kind: OptionKind) -> Vec<&'a str> {
    let q = query.trim().to_lowercase();
    options
        .iter()
        .map(|s| s.as_str())
        .filter(|s| {
            let label = option_label(kind, s);
            // Hide leftovers that would still read as English (user: 不要英文).
            if kind == OptionKind::Timezone && !is_zh_display(&label) {
                return false;
            }
            if q.is_empty() {
                return true;
            }
            s.to_lowercase().contains(&q) || label.to_lowercase().contains(&q)
        })
        .collect()
}

/// True when the display label is Chinese / digit / punctuation (UTC allowed).
fn is_zh_display(label: &str) -> bool {
    let t = label.trim();
    if t.is_empty() {
        return false;
    }
    if t.eq_ignore_ascii_case("utc") {
        return true;
    }
    !t.chars().any(|c| c.is_ascii_alphabetic())
}

/// Ensure the live draft value is always selectable even if not in the list.
pub fn with_current(options: &[String], current: &str) -> Vec<String> {
    let mut out: Vec<String> = options.to_vec();
    let cur = current.trim();
    if !cur.is_empty() && !out.iter().any(|s| s.eq_ignore_ascii_case(cur)) {
        out.insert(0, cur.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_show_only_cn_us_and_timezone() {
        assert_eq!(region_label("CN"), "中国");
        assert_eq!(region_label("US"), "美国");
        assert_eq!(region_label("JP"), "JP");
        assert_eq!(locale_label("zh-CN"), "zh-CN");
        assert_eq!(locale_label("en-US"), "en-US");
        assert_eq!(timezone_label("China Standard Time"), "中国标准时间");
        assert_eq!(timezone_label("pacific standard time"), "太平洋时间");
        assert_eq!(timezone_label("Atlantic Standard Time"), "大西洋时间");
        assert_eq!(timezone_label("SA Pacific Standard Time"), "南美太平洋时间");
        assert_eq!(
            timezone_label("E. South America Standard Time"),
            "南美东部时间"
        );
        assert_eq!(timezone_label("Pacific SA Standard Time"), "南美西部时间");
        assert_eq!(timezone_label("GMT Standard Time"), "英国时间");
        assert_eq!(timezone_label("Asia/Shanghai"), "上海");
        assert_eq!(timezone_label("America/Los_Angeles"), "洛杉矶");
        // Host leftovers never look like raw English Standard Time ids.
        assert!(!timezone_label("W. Europe Standard Time").contains("Standard"));
        assert!(!timezone_label("Easter Island Standard Time").contains("Standard"));
        assert!(timezone_label("W. Europe Standard Time").contains("西欧"));
        // Unmapped English place names are hidden from the list, not shown raw.
        let host = vec![
            "Pacific Standard Time".to_string(),
            "Easter Island Standard Time".to_string(),
        ];
        assert_eq!(
            filter_options(&host, "", OptionKind::Timezone),
            vec!["Pacific Standard Time"]
        );
    }

    #[test]
    fn filter_matches_cn_us_and_timezone() {
        let regions = vec!["CN".to_string(), "US".to_string()];
        assert_eq!(
            filter_options(&regions, "中国", OptionKind::Region),
            vec!["CN"]
        );
        assert_eq!(
            filter_options(&regions, "美国", OptionKind::Region),
            vec!["US"]
        );

        let opts = vec![
            "Asia/Shanghai".to_string(),
            "America/New_York".to_string(),
        ];
        assert_eq!(
            filter_options(&opts, "上海", OptionKind::Timezone),
            vec!["Asia/Shanghai"]
        );
        assert_eq!(
            filter_options(&opts, "shanghai", OptionKind::Timezone),
            vec!["Asia/Shanghai"]
        );
    }

    #[test]
    fn with_current_prepends_missing_value() {
        let opts = vec!["US".to_string()];
        assert_eq!(with_current(&opts, "JP")[0], "JP");
        assert_eq!(with_current(&opts, "US").len(), 1);
    }
}
