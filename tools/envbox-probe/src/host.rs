//! Host-side readers for GEO / LOCALE / LANGUAGE / TIMEZONE / DNS / ENV.
//! Read-only: never writes Host configuration.

use crate::{
    Field, HostSnapshot, Section, SECTION_DNS, SECTION_ENV, SECTION_GEO, SECTION_LANGUAGE,
    SECTION_LOCALE, SECTION_TIMEZONE,
};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::Win32::Globalization::{
    GetLocaleInfoEx, GetLocaleInfoW, GetSystemDefaultLCID, GetSystemDefaultLocaleName,
    GetSystemDefaultUILanguage, GetProcessPreferredUILanguages,
    GetSystemPreferredUILanguages, GetThreadPreferredUILanguages,
    GetUserDefaultGeoName, GetUserDefaultLCID, GetUserDefaultLocaleName,
    GetUserDefaultUILanguage, GetUserGeoID, GetUserPreferredUILanguages,
    LOCALE_SNAME, MUI_LANGUAGE_NAME, SYSGEOCLASS,
};
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GetNetworkParams, GET_ADAPTERS_ADDRESSES_FLAGS,
};
use windows::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation,
    GetTimeZoneInformation, GetTimeZoneInformationForYear,
    SystemTimeToTzSpecificLocalTime, SystemTimeToTzSpecificLocalTimeEx,
    TIME_ZONE_INFORMATION, TzSpecificLocalTimeToSystemTime,
    TzSpecificLocalTimeToSystemTimeEx,
};
use windows::core::PWSTR;

const LOCALE_NAME_MAX_LENGTH: usize = 85;

pub fn collect() -> HostSnapshot {
    HostSnapshot {
        sections: vec![
            collect_geo(),
            collect_locale(),
            collect_language(),
            collect_timezone(),
            collect_dns(),
            collect_env(),
        ],
    }
}

fn field(name: &str, value: impl Into<String>) -> Field {
    Field {
        name: name.to_string(),
        value: value.into(),
    }
}

fn collect_geo() -> Section {
    let mut fields = Vec::new();
    unsafe {
        let mut buf = [0u16; 128];
        let written = GetUserDefaultGeoName(&mut buf);
        fields.push(field(
            "GetUserDefaultGeoName",
            if written > 0 {
                wstr_from_until_nul(&buf)
            } else {
                "<error>".into()
            },
        ));
        let geo_id = GetUserGeoID(SYSGEOCLASS(16)); // GEOCLASS_NATION
        fields.push(field("GetUserGeoID", geo_id.to_string()));
    }
    Section {
        title: SECTION_GEO.to_string(),
        fields,
    }
}

fn collect_locale() -> Section {
    let mut fields = Vec::new();
    unsafe {
        let mut user = [0u16; LOCALE_NAME_MAX_LENGTH];
        let user_ok = GetUserDefaultLocaleName(&mut user);
        fields.push(field(
            "GetUserDefaultLocaleName",
            if user_ok > 0 {
                wstr_from_until_nul(&user)
            } else {
                "<error>".into()
            },
        ));

        let mut sys = [0u16; LOCALE_NAME_MAX_LENGTH];
        let sys_ok = GetSystemDefaultLocaleName(&mut sys);
        fields.push(field(
            "GetSystemDefaultLocaleName",
            if sys_ok > 0 {
                wstr_from_until_nul(&sys)
            } else {
                "<error>".into()
            },
        ));

        let lcid = GetUserDefaultLCID();
        fields.push(field("GetUserDefaultLCID", lcid.to_string()));
        let sys_lcid = GetSystemDefaultLCID();
        fields.push(field("GetSystemDefaultLCID", sys_lcid.to_string()));

        let mut sname = [0u16; 85];
        let sname_ok = GetLocaleInfoEx(
            windows::core::PCWSTR::null(),
            LOCALE_SNAME,
            Some(&mut sname),
        );
        fields.push(field(
            "GetLocaleInfoEx_SNAME",
            if sname_ok > 0 {
                wstr_from_until_nul(&sname)
            } else {
                "<error>".into()
            },
        ));

        let mut sname_w = [0u16; 85];
        let sname_w_ok = GetLocaleInfoW(lcid, LOCALE_SNAME, Some(&mut sname_w));
        fields.push(field(
            "GetLocaleInfoW_SNAME",
            if sname_w_ok > 0 {
                wstr_from_until_nul(&sname_w)
            } else {
                "<error>".into()
            },
        ));
    }
    Section {
        title: SECTION_LOCALE.to_string(),
        fields,
    }
}

fn collect_language() -> Section {
    let mut fields = Vec::new();
    unsafe {
        let user_ui = GetUserDefaultUILanguage();
        fields.push(field(
            "GetUserDefaultUILanguage",
            format!("{user_ui:#06x}"),
        ));
        let sys_ui = GetSystemDefaultUILanguage();
        fields.push(field(
            "GetSystemDefaultUILanguage",
            format!("{sys_ui:#06x}"),
        ));

        push_preferred_ui(&mut fields, "GetUserPreferredUILanguages", |count, buf, size| {
            GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size)
        });
        push_preferred_ui(&mut fields, "GetSystemPreferredUILanguages", |count, buf, size| {
            GetSystemPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size)
        });
        push_preferred_ui(&mut fields, "GetThreadPreferredUILanguages", |count, buf, size| {
            GetThreadPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size)
        });
        push_preferred_ui(&mut fields, "GetProcessPreferredUILanguages", |count, buf, size| {
            GetProcessPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size)
        });
        push_preferred_ui(&mut fields, "GetUserPreferredUILanguages_ID", |count, buf, size| {
            GetUserPreferredUILanguages(0x4, count, buf, size) // MUI_LANGUAGE_ID
        });
    }
    Section {
        title: SECTION_LANGUAGE.to_string(),
        fields,
    }
}

fn push_preferred_ui(
    fields: &mut Vec<Field>,
    name: &str,
    mut call: impl FnMut(&mut u32, PWSTR, &mut u32) -> windows::core::Result<()>,
) {
    let mut count = 0u32;
    let mut buffer_size = 0u32;
    let empty = PWSTR::null();
    let _ = call(&mut count, empty, &mut buffer_size);
    if buffer_size > 0 {
        let mut buffer = vec![0u16; buffer_size as usize];
        let mut count2 = count;
        let mut size2 = buffer_size;
        let ptr = PWSTR(buffer.as_mut_ptr());
        match call(&mut count2, ptr, &mut size2) {
            Ok(()) => fields.push(field(name, parse_multi_sz(&buffer))),
            Err(_) => fields.push(field(name, "<error>")),
        }
    } else {
        fields.push(field(name, "<empty>"));
    }
}

fn format_system_time(t: &windows::Win32::Foundation::SYSTEMTIME) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

fn collect_timezone() -> Section {
    let mut fields = Vec::new();
    unsafe {
        let mut tzi = std::mem::MaybeUninit::<DYNAMIC_TIME_ZONE_INFORMATION>::zeroed();
        let id = GetDynamicTimeZoneInformation(tzi.as_mut_ptr());
        // TIME_ZONE_ID_INVALID == u32::MAX
        if id == u32::MAX {
            fields.push(field("GetDynamicTimeZoneInformation", "<error>"));
        } else {
            let tzi = tzi.assume_init();
            fields.push(field(
                "GetDynamicTimeZoneInformation",
                wide_to_string(&tzi.TimeZoneKeyName),
            ));
            fields.push(field("Bias", tzi.Bias.to_string()));
        }

        let mut classic = std::mem::MaybeUninit::<TIME_ZONE_INFORMATION>::zeroed();
        let id2 = GetTimeZoneInformation(classic.as_mut_ptr());
        if id2 == u32::MAX {
            fields.push(field("GetTimeZoneInformation", "<error>"));
        } else {
            let tzi = classic.assume_init();
            fields.push(field(
                "GetTimeZoneInformation",
                wide_to_string(&tzi.StandardName),
            ));
            fields.push(field("GetTimeZoneInformation_Bias", tzi.Bias.to_string()));
        }

        // Fixed UTC 2024-01-15 12:00:00 (independent fixture for conversion).
        let utc = windows::Win32::Foundation::SYSTEMTIME {
            wYear: 2024,
            wMonth: 1,
            wDay: 15,
            wDayOfWeek: 1,
            wHour: 12,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        let mut local = windows::Win32::Foundation::SYSTEMTIME::default();
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok() {
            fields.push(field("SystemTimeToTzSpecificLocalTime", format_system_time(&local)));
        } else {
            fields.push(field("SystemTimeToTzSpecificLocalTime", "<error>"));
        }

        let mut back = windows::Win32::Foundation::SYSTEMTIME::default();
        if TzSpecificLocalTimeToSystemTime(None, &local, &mut back).is_ok() {
            fields.push(field("TzSpecificLocalTimeToSystemTime", format_system_time(&back)));
        } else {
            fields.push(field("TzSpecificLocalTimeToSystemTime", "<error>"));
        }

        let mut local_ex = windows::Win32::Foundation::SYSTEMTIME::default();
        if SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local_ex).is_ok() {
            fields.push(field(
                "SystemTimeToTzSpecificLocalTimeEx",
                format_system_time(&local_ex),
            ));
        } else {
            fields.push(field("SystemTimeToTzSpecificLocalTimeEx", "<error>"));
        }

        let mut back_ex = windows::Win32::Foundation::SYSTEMTIME::default();
        if TzSpecificLocalTimeToSystemTimeEx(None, &local_ex, &mut back_ex).is_ok() {
            fields.push(field(
                "TzSpecificLocalTimeToSystemTimeEx",
                format_system_time(&back_ex),
            ));
        } else {
            fields.push(field("TzSpecificLocalTimeToSystemTimeEx", "<error>"));
        }

        let mut year_tzi = TIME_ZONE_INFORMATION::default();
        if GetTimeZoneInformationForYear(0, None, &mut year_tzi).is_ok() {
            fields.push(field(
                "GetTimeZoneInformationForYear",
                wide_to_string(&year_tzi.StandardName),
            ));
        } else {
            fields.push(field("GetTimeZoneInformationForYear", "<error>"));
        }
    }
    Section {
        title: SECTION_TIMEZONE.to_string(),
        fields,
    }
}

fn collect_dns() -> Section {
    let mut fields = Vec::new();
    unsafe {
        let mut size = 0u32;
        let _ = GetNetworkParams(None, &mut size);
        if size > 0 {
            let mut buf = vec![0u8; size as usize];
            let ptr = buf.as_mut_ptr()
                as *mut windows::Win32::NetworkManagement::IpHelper::FIXED_INFO_W2KSP1;
            let status = GetNetworkParams(Some(ptr), &mut size);
            if status.0 == 0 {
                fields.push(field("GetNetworkParams", read_dns_server_list(ptr)));
            } else {
                fields.push(field("GetNetworkParams", format!("error={}", status.0)));
            }
        } else {
            fields.push(field("GetNetworkParams", "<empty>"));
        }

        let mut buflen = 0u32;
        let _ = GetAdaptersAddresses(0, GET_ADAPTERS_ADDRESSES_FLAGS(0), None, None, &mut buflen);
        if buflen > 0 {
            let mut buffer = vec![0u8; buflen as usize];
            let addr = buffer.as_mut_ptr() as *mut _;
            let status = GetAdaptersAddresses(
                0,
                GET_ADAPTERS_ADDRESSES_FLAGS(0),
                None,
                Some(addr),
                &mut buflen,
            );
            fields.push(field("GetAdaptersAddresses", format!("status={status}")));
        } else {
            fields.push(field("GetAdaptersAddresses", "<empty>"));
        }
    }
    Section {
        title: SECTION_DNS.to_string(),
        fields,
    }
}

unsafe fn read_dns_server_list(
    info: *const windows::Win32::NetworkManagement::IpHelper::FIXED_INFO_W2KSP1,
) -> String {
    if info.is_null() {
        return "<null>".into();
    }
    let info = &*info;
    let mut servers = Vec::new();
    push_ip_string(&mut servers, &info.DnsServerList);
    let mut next = info.DnsServerList.Next;
    while !next.is_null() {
        let node = &*next;
        push_ip_string(&mut servers, node);
        next = node.Next;
    }
    if servers.is_empty() {
        "<none>".into()
    } else {
        servers.join(", ")
    }
}

unsafe fn push_ip_string(
    out: &mut Vec<String>,
    addr: &windows::Win32::NetworkManagement::IpHelper::IP_ADDR_STRING,
) {
    let s = std::ffi::CStr::from_ptr(addr.IpAddress.String.as_ptr() as *const _)
        .to_string_lossy()
        .into_owned();
    let s = s.trim().to_string();
    if !s.is_empty() {
        out.push(s);
    }
}

fn collect_env() -> Section {
    // Full ENV snapshot (sorted). Always include comparison keys even when unset.
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    for key in ["LANG", "LC_ALL", "TZ", "PATH"] {
        if !vars.iter().any(|(name, _)| name.eq_ignore_ascii_case(key)) {
            vars.push((key.to_string(), String::from("<unset>")));
        }
    }
    vars.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
    vars.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    let fields = vars
        .into_iter()
        .map(|(name, value)| field(&name, value))
        .collect();
    Section {
        title: SECTION_ENV.to_string(),
        fields,
    }
}

fn wstr_from_until_nul(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
        .to_string_lossy()
        .into_owned()
}

fn wide_to_string(buf: &[u16]) -> String {
    wstr_from_until_nul(buf)
}

fn parse_multi_sz(buffer: &[u16]) -> String {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < buffer.len() {
        if buffer[i] == 0 {
            if start == i {
                break;
            }
            let s = OsString::from_wide(&buffer[start..i])
                .to_string_lossy()
                .into_owned();
            parts.push(s);
            start = i + 1;
        }
        i += 1;
    }
    parts.join(", ")
}
