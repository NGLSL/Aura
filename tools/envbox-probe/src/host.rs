//! Host-side readers for GEO / LOCALE / LANGUAGE / TIMEZONE / DNS / ENV.
//! Read-only: never writes Host configuration.

use crate::{
    Field, HostSnapshot, Section, SECTION_DNS, SECTION_ENV, SECTION_GEO, SECTION_LANGUAGE,
    SECTION_LOCALE, SECTION_REGISTRY, SECTION_TIMEZONE,
};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Globalization::Calendar;
use windows::Win32::Globalization::{
    GetACP, GetGeoInfoA, GetGeoInfoW, GetLocaleInfoA, GetLocaleInfoEx, GetLocaleInfoW, GetOEMCP,
    GetProcessPreferredUILanguages, GetSystemDefaultLCID, GetSystemDefaultLangID,
    GetSystemDefaultLocaleName, GetSystemDefaultUILanguage, GetSystemPreferredUILanguages,
    GetThreadLocale, GetThreadPreferredUILanguages,
    GetUserDefaultGeoName, GetUserDefaultLCID, GetUserDefaultLocaleName, GetUserDefaultUILanguage,
    GetUserDefaultLangID, GetUserGeoID, GetUserPreferredUILanguages, MultiByteToWideChar,
    WideCharToMultiByte, CP_ACP, CP_OEMCP, CP_THREAD_ACP, GEO_ISO2, LOCALE_IDEFAULTANSICODEPAGE,
    LOCALE_IDEFAULTCODEPAGE, LOCALE_SNAME, MUI_LANGUAGE_NAME, MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
    SYSGEOCLASS,
};
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GetNetworkParams, GET_ADAPTERS_ADDRESSES_FLAGS,
};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::System::Time::{
    GetDynamicTimeZoneInformation, GetTimeZoneInformation, GetTimeZoneInformationForYear,
    SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime, SystemTimeToTzSpecificLocalTimeEx,
    TzSpecificLocalTimeToSystemTime, TzSpecificLocalTimeToSystemTimeEx,
    DYNAMIC_TIME_ZONE_INFORMATION, TIME_ZONE_INFORMATION,
};
use windows::Win32::System::WinRT::RoActivateInstance;

const LOCALE_NAME_MAX_LENGTH: usize = 85;

pub fn collect() -> HostSnapshot {
    HostSnapshot {
        sections: vec![
            collect_geo(),
            collect_locale(),
            collect_language(),
            collect_timezone(),
            collect_dns(),
            collect_registry(),
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
        let mut iso2_w = [0u16; 8];
        let iso2_w_len = GetGeoInfoW(geo_id, GEO_ISO2, Some(&mut iso2_w), 0);
        fields.push(field(
            "GetGeoInfoW_ISO2",
            if iso2_w_len > 0 {
                wstr_from_until_nul(&iso2_w)
            } else {
                "<error>".into()
            },
        ));
        let mut iso2_a = [0u8; 8];
        let iso2_a_len = GetGeoInfoA(geo_id, GEO_ISO2, Some(&mut iso2_a), 0);
        fields.push(field(
            "GetGeoInfoA_ISO2",
            if iso2_a_len > 0 {
                astr_from_until_nul(&iso2_a)
            } else {
                "<error>".into()
            },
        ));
    }
    Section {
        title: SECTION_GEO.to_string(),
        fields,
    }
}

fn collect_locale() -> Section {
    let mut fields = Vec::new();
    unsafe {
        // Chromium/ICU also reads these process code pages directly. The
        // Runtime derives them from the Profile locale and routes CP_ACP /
        // CP_OEMCP conversions through the same pages.
        fields.push(field("GetACP", GetACP().to_string()));
        fields.push(field("GetOEMCP", GetOEMCP().to_string()));
        fields.push(field("GetThreadLocale", GetThreadLocale().to_string()));
        fields.push(field("GetUserDefaultLangID", GetUserDefaultLangID().to_string()));
        fields.push(field(
            "GetSystemDefaultLangID",
            GetSystemDefaultLangID().to_string(),
        ));
        fields.push(field(
            "GetLocaleInfoEx_IDEFAULTANSICODEPAGE",
            locale_info_string(LOCALE_IDEFAULTANSICODEPAGE),
        ));
        fields.push(field(
            "GetLocaleInfoEx_IDEFAULTCODEPAGE",
            locale_info_string(LOCALE_IDEFAULTCODEPAGE),
        ));
        fields.push(field(
            "WideCharToMultiByte_CP_ACP_e_acute",
            wide_char_to_code_page(CP_ACP, 'é'),
        ));
        fields.push(field(
            "WideCharToMultiByte_CP_OEMCP_e_acute",
            wide_char_to_code_page(CP_OEMCP, 'é'),
        ));
        fields.push(field(
            "WideCharToMultiByte_CP_THREAD_ACP_e_acute",
            wide_char_to_code_page(CP_THREAD_ACP, 'é'),
        ));
        fields.push(field(
            "MultiByteToWideChar_CP_ACP_e9",
            code_page_to_wide(CP_ACP, &[0xe9]),
        ));
        fields.push(field(
            "MultiByteToWideChar_CP_OEMCP_82",
            code_page_to_wide(CP_OEMCP, &[0x82]),
        ));
        fields.push(field(
            "MultiByteToWideChar_CP_THREAD_ACP_e9",
            code_page_to_wide(CP_THREAD_ACP, &[0xe9]),
        ));
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

        let mut sname_a = [0u8; 85];
        let sname_a_ok = GetLocaleInfoA(lcid, LOCALE_SNAME, Some(&mut sname_a));
        fields.push(field(
            "GetLocaleInfoA_SNAME",
            if sname_a_ok > 0 {
                astr_from_until_nul(&sname_a)
            } else {
                "<error>".into()
            },
        ));

        // Query the default alias directly; the explicit LCID above can hide a missing A hook.
        let mut default_sname_a = [0u8; 85];
        let default_sname_a_ok = GetLocaleInfoA(0x0400, LOCALE_SNAME, Some(&mut default_sname_a));
        fields.push(field(
            "GetLocaleInfoA_USER_DEFAULT_SNAME",
            if default_sname_a_ok > 0 {
                astr_from_until_nul(&default_sname_a)
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

fn locale_info_string(lctype: u32) -> String {
    unsafe {
        let mut value = [0u16; 16];
        let result = GetLocaleInfoEx(windows::core::PCWSTR::null(), lctype, Some(&mut value));
        if result > 0 {
            wstr_from_until_nul(&value)
        } else {
            "<error>".into()
        }
    }
}

fn wide_char_to_code_page(code_page: u32, character: char) -> String {
    let mut source_buffer = [0u16; 2];
    let source = character.encode_utf16(&mut source_buffer);
    let mut output = [0u8; 8];
    let written =
        unsafe { WideCharToMultiByte(code_page, 0, &source, Some(&mut output), None, None) };
    if written <= 0 {
        return "<error>".into();
    }
    output[..written as usize]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn code_page_to_wide(code_page: u32, bytes: &[u8]) -> String {
    let mut output = [0u16; 8];
    let written = unsafe {
        MultiByteToWideChar(
            code_page,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(&mut output),
        )
    };
    if written <= 0 {
        return "<error>".into();
    }
    String::from_utf16_lossy(&output[..written as usize])
}

fn collect_language() -> Section {
    let mut fields = Vec::new();
    unsafe {
        let user_ui = GetUserDefaultUILanguage();
        fields.push(field("GetUserDefaultUILanguage", format!("{user_ui:#06x}")));
        let sys_ui = GetSystemDefaultUILanguage();
        fields.push(field(
            "GetSystemDefaultUILanguage",
            format!("{sys_ui:#06x}"),
        ));

        push_preferred_ui(
            &mut fields,
            "GetUserPreferredUILanguages",
            |count, buf, size| GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size),
        );
        push_preferred_ui(
            &mut fields,
            "GetSystemPreferredUILanguages",
            |count, buf, size| GetSystemPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size),
        );
        push_preferred_ui(
            &mut fields,
            "GetThreadPreferredUILanguages",
            |count, buf, size| GetThreadPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size),
        );
        push_preferred_ui(
            &mut fields,
            "GetProcessPreferredUILanguages",
            |count, buf, size| GetProcessPreferredUILanguages(MUI_LANGUAGE_NAME, count, buf, size),
        );
        push_preferred_ui(
            &mut fields,
            "GetUserPreferredUILanguages_ID",
            |count, buf, size| {
                GetUserPreferredUILanguages(0x4, count, buf, size) // MUI_LANGUAGE_ID
            },
        );
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

fn local_times_match_within_two_seconds(
    a: &windows::Win32::Foundation::SYSTEMTIME,
    b: &windows::Win32::Foundation::SYSTEMTIME,
) -> Option<bool> {
    let mut a_filetime = windows::Win32::Foundation::FILETIME::default();
    let mut b_filetime = windows::Win32::Foundation::FILETIME::default();
    unsafe {
        SystemTimeToFileTime(a, &mut a_filetime).ok()?;
        SystemTimeToFileTime(b, &mut b_filetime).ok()?;
    }
    let ticks = |ft: windows::Win32::Foundation::FILETIME| {
        (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
    };
    Some(ticks(a_filetime).abs_diff(ticks(b_filetime)) <= 2 * 10_000_000)
}

fn collect_timezone() -> Section {
    let mut fields = Vec::new();
    fields.push(field(
        "WinRT_Calendar_GetTimeZone",
        match Calendar::new().and_then(|calendar| calendar.GetTimeZone()) {
            Ok(timezone) => timezone.to_string(),
            Err(error) => format!("<error {:#010x}>", error.code().0 as u32),
        },
    ));
    fields.push(field(
        "WinRT_Calendar_ChangedTimeZone",
        match Calendar::new().and_then(|calendar| {
            calendar.ChangeTimeZone(&HSTRING::from("Europe/London"))?;
            calendar.GetTimeZone()
        }) {
            Ok(timezone) => timezone.to_string(),
            Err(error) => format!("<error {:#010x}>", error.code().0 as u32),
        },
    ));
    fields.push(field(
        "WinRT_RoActivateInstance_GetTimeZone",
        match unsafe { RoActivateInstance(&HSTRING::from("Windows.Globalization.Calendar")) }
            .and_then(|instance| instance.cast::<Calendar>())
            .and_then(|calendar| calendar.GetTimeZone())
        {
            Ok(timezone) => timezone.to_string(),
            Err(error) => format!("<error {:#010x}>", error.code().0 as u32),
        },
    ));
    unsafe {
        // Sample UTC and local time together so the two local-time paths can be compared.
        let now_utc = GetSystemTime();
        let now_local = GetLocalTime();
        fields.push(field("GetSystemTime", format_system_time(&now_utc)));
        fields.push(field("GetLocalTime", format_system_time(&now_local)));
        let mut converted_now = windows::Win32::Foundation::SYSTEMTIME::default();
        if SystemTimeToTzSpecificLocalTime(None, &now_utc, &mut converted_now).is_ok() {
            fields.push(field(
                "SystemTimeToTzSpecificLocalTime_Now",
                format_system_time(&converted_now),
            ));
            fields.push(field(
                "GetLocalTime_MatchesProfileConversion",
                match local_times_match_within_two_seconds(&now_local, &converted_now) {
                    Some(matches) => matches.to_string(),
                    None => "<error>".into(),
                },
            ));
        } else {
            fields.push(field("SystemTimeToTzSpecificLocalTime_Now", "<error>"));
            fields.push(field("GetLocalTime_MatchesProfileConversion", "<error>"));
        }

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
            fields.push(field(
                "SystemTimeToTzSpecificLocalTime",
                format_system_time(&local),
            ));
        } else {
            fields.push(field("SystemTimeToTzSpecificLocalTime", "<error>"));
        }

        // Fixed UTC date boundary: Pacific time is still the previous day.
        let boundary_utc = windows::Win32::Foundation::SYSTEMTIME {
            wYear: 2024,
            wMonth: 1,
            wDay: 1,
            wDayOfWeek: 1,
            wHour: 3,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        let mut boundary_local = windows::Win32::Foundation::SYSTEMTIME::default();
        if SystemTimeToTzSpecificLocalTime(None, &boundary_utc, &mut boundary_local).is_ok() {
            fields.push(field(
                "SystemTimeToTzSpecificLocalTime_DateBoundary",
                format_system_time(&boundary_local),
            ));
        } else {
            fields.push(field(
                "SystemTimeToTzSpecificLocalTime_DateBoundary",
                "<error>",
            ));
        }

        let mut back = windows::Win32::Foundation::SYSTEMTIME::default();
        if TzSpecificLocalTimeToSystemTime(None, &local, &mut back).is_ok() {
            fields.push(field(
                "TzSpecificLocalTimeToSystemTime",
                format_system_time(&back),
            ));
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
            if status == 0 {
                fields.push(field("GetAdaptersAddresses", read_adapter_dns(addr)));
            } else {
                fields.push(field("GetAdaptersAddresses", format!("status={status}")));
            }
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

unsafe fn read_adapter_dns(
    first: *mut windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_ADDRESSES_LH,
) -> String {
    if first.is_null() {
        return "<null>".into();
    }
    let mut servers = Vec::new();
    let mut adapter: *const windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_ADDRESSES_LH =
        first;
    while !adapter.is_null() {
        let a = &*adapter;
        let mut dns = a.FirstDnsServerAddress;
        while !dns.is_null() {
            let d = &*dns;
            let sa = d.Address.lpSockaddr;
            if !sa.is_null() {
                let fam = (*sa).sa_family;
                if fam == windows::Win32::Networking::WinSock::ADDRESS_FAMILY(2) {
                    // AF_INET
                    let v4 = &*(sa as *const windows::Win32::Networking::WinSock::SOCKADDR_IN);
                    let ip = v4.sin_addr;
                    let bytes = [
                        (ip.S_un.S_addr & 0xff) as u8,
                        ((ip.S_un.S_addr >> 8) & 0xff) as u8,
                        ((ip.S_un.S_addr >> 16) & 0xff) as u8,
                        ((ip.S_un.S_addr >> 24) & 0xff) as u8,
                    ];
                    // Network byte order: already big-endian in S_addr on Windows...
                    // Use WSAAddressToString-free path: inet_ntoa style from network order.
                    let s = format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3]);
                    if !servers.contains(&s) {
                        servers.push(s);
                    }
                } else if fam == windows::Win32::Networking::WinSock::ADDRESS_FAMILY(23) {
                    // AF_INET6
                    let v6 = &*(sa as *const windows::Win32::Networking::WinSock::SOCKADDR_IN6);
                    let b = v6.sin6_addr.u.Byte;
                    let s = format!(
                        "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                        u16::from_be_bytes([b[0], b[1]]),
                        u16::from_be_bytes([b[2], b[3]]),
                        u16::from_be_bytes([b[4], b[5]]),
                        u16::from_be_bytes([b[6], b[7]]),
                        u16::from_be_bytes([b[8], b[9]]),
                        u16::from_be_bytes([b[10], b[11]]),
                        u16::from_be_bytes([b[12], b[13]]),
                        u16::from_be_bytes([b[14], b[15]]),
                    );
                    if !servers.contains(&s) {
                        servers.push(s);
                    }
                }
            }
            dns = d.Next;
        }
        adapter = a.Next;
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

fn collect_registry() -> Section {
    let mut fields = Vec::new();
    push_reg_sz(
        &mut fields,
        "HKCU_International_LocaleName",
        windows::Win32::System::Registry::HKEY_CURRENT_USER,
        "Control Panel\\International",
        "LocaleName",
    );
    push_reg_sz(
        &mut fields,
        "HKCU_International_Locale",
        windows::Win32::System::Registry::HKEY_CURRENT_USER,
        "Control Panel\\International",
        "Locale",
    );
    push_reg_sz(
        &mut fields,
        "HKLM_TimeZone_TimeZoneKeyName",
        windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation",
        "TimeZoneKeyName",
    );
    push_reg_sz_a(
        &mut fields,
        "HKLM_TimeZone_TimeZoneKeyName_A",
        windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation",
        "TimeZoneKeyName",
    );
    let one_shot_a = fields
        .iter()
        .find(|field| field.name == "HKLM_TimeZone_TimeZoneKeyName_A")
        .map(|field| field.value.as_str())
        .unwrap_or("<missing>");
    let query_contract = query_timezone_key_name_a_contract(one_shot_a);
    fields.push(field(
        "HKLM_TimeZone_TimeZoneKeyName_A_QueryContract",
        query_contract,
    ));
    push_reg_sz(
        &mut fields,
        "HKLM_TimeZone_StandardName",
        windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation",
        "StandardName",
    );
    // Nested open: HKCU\\Control Panel then International\\LocaleName via handle.
    push_reg_nested(&mut fields, "Nested_International_LocaleName");
    // Non-whitelist key must stay Host (not a Registry Sandbox).
    push_reg_sz(
        &mut fields,
        "HKLM_WindowsNT_CurrentVersion",
        windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
        "CurrentVersion",
    );
    Section {
        title: SECTION_REGISTRY.to_string(),
        fields,
    }
}

fn push_reg_nested(fields: &mut Vec<Field>, name: &str) {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ,
    };
    let mut mid = windows::Win32::System::Registry::HKEY::default();
    let mut leaf = windows::Win32::System::Registry::HKEY::default();
    let sub1: Vec<u16> = "Control Panel\0".encode_utf16().collect();
    let sub2: Vec<u16> = "International\0".encode_utf16().collect();
    let val: Vec<u16> = "LocaleName\0".encode_utf16().collect();
    let mut buf = [0u16; 128];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        let s1 = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(sub1.as_ptr()),
            0,
            KEY_READ,
            &mut mid,
        );
        if s1.0 != 0 {
            fields.push(field(name, "<error-open1>"));
            return;
        }
        let s2 = RegOpenKeyExW(
            mid,
            windows::core::PCWSTR(sub2.as_ptr()),
            0,
            KEY_READ,
            &mut leaf,
        );
        if s2.0 != 0 {
            let _ = RegCloseKey(mid);
            fields.push(field(name, "<error-open2>"));
            return;
        }
        let sq = RegQueryValueExW(
            leaf,
            windows::core::PCWSTR(val.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        );
        let _ = RegCloseKey(leaf);
        let _ = RegCloseKey(mid);
        if sq.0 == 0 {
            fields.push(field(name, wstr_from_until_nul(&buf)));
        } else {
            fields.push(field(name, "<error-query>"));
        }
    }
}

fn push_reg_sz(
    fields: &mut Vec<Field>,
    name: &str,
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value: &str,
) {
    use windows::Win32::System::Registry::{RegGetValueW, REG_VALUE_TYPE, RRF_RT_REG_SZ};
    let sub: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let val: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let mut ty = REG_VALUE_TYPE(0);
    let status = unsafe {
        RegGetValueW(
            root,
            windows::core::PCWSTR(sub.as_ptr()),
            windows::core::PCWSTR(val.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut ty as *mut REG_VALUE_TYPE),
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if status.0 == 0 {
        fields.push(field(name, wstr_from_until_nul(&buf)));
    } else {
        fields.push(field(name, "<error>"));
    }
}

fn push_reg_sz_a(
    fields: &mut Vec<Field>,
    name: &str,
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value: &str,
) {
    use windows::Win32::System::Registry::{RegGetValueA, REG_VALUE_TYPE, RRF_RT_REG_SZ};
    let sub: Vec<u8> = subkey.bytes().chain(std::iter::once(0)).collect();
    let val: Vec<u8> = value.bytes().chain(std::iter::once(0)).collect();
    let mut buf = [0u8; 256];
    let mut size = buf.len() as u32;
    let mut ty = REG_VALUE_TYPE(0);
    let status = unsafe {
        RegGetValueA(
            root,
            windows::core::PCSTR(sub.as_ptr()),
            windows::core::PCSTR(val.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut ty as *mut REG_VALUE_TYPE),
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if status.0 == 0 {
        fields.push(field(name, astr_from_until_nul(&buf)));
    } else {
        fields.push(field(name, "<error>"));
    }
}

fn query_timezone_key_name_a_contract(one_shot: &str) -> String {
    use windows::Win32::Foundation::ERROR_MORE_DATA;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExA, RegQueryValueExA, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
        REG_VALUE_TYPE,
    };

    struct OpenKey(HKEY);
    impl Drop for OpenKey {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    let mut raw = HKEY::default();
    let status = unsafe {
        RegOpenKeyExA(
            HKEY_LOCAL_MACHINE,
            windows::core::s!("SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation"),
            0,
            KEY_READ,
            &mut raw,
        )
    };
    if status.0 != 0 {
        return format!("<error-open {}>", status.0);
    }
    let key = OpenKey(raw);
    let name = windows::core::s!("TimeZoneKeyName");
    let mut ty = REG_VALUE_TYPE(0);
    let mut required = 0u32;
    let status =
        unsafe { RegQueryValueExA(key.0, name, None, Some(&mut ty), None, Some(&mut required)) };
    if status.0 != 0 || ty != REG_SZ || required < 2 {
        return format!(
            "<error-size status={} type={} bytes={}>",
            status.0, ty.0, required
        );
    }

    let mut short = vec![0u8; required as usize - 1];
    let mut short_size = short.len() as u32;
    let short_status = unsafe {
        RegQueryValueExA(
            key.0,
            name,
            None,
            Some(&mut ty),
            Some(short.as_mut_ptr()),
            Some(&mut short_size),
        )
    };
    if short_status != ERROR_MORE_DATA || ty != REG_SZ || short_size != required {
        return format!(
            "<error-short status={} type={} bytes={} expected={}>",
            short_status.0, ty.0, short_size, required
        );
    }

    let mut exact = vec![0u8; required as usize];
    let mut exact_size = exact.len() as u32;
    let exact_status = unsafe {
        RegQueryValueExA(
            key.0,
            name,
            None,
            Some(&mut ty),
            Some(exact.as_mut_ptr()),
            Some(&mut exact_size),
        )
    };
    if exact_status.0 != 0 || ty != REG_SZ || exact_size != required {
        return format!(
            "<error-exact status={} type={} bytes={} expected={}>",
            exact_status.0, ty.0, exact_size, required
        );
    }
    let exact_value = astr_from_until_nul(&exact);
    if exact.last() != Some(&0) || exact_value != one_shot {
        return "<error-value-mismatch>".into();
    }
    if let Ok(expected_profile) = std::env::var("ENVBOX_TZ_WINDOWS") {
        if exact_value != expected_profile {
            return format!(
                "<error-profile-mismatch expected={expected_profile} actual={exact_value}>"
            );
        }
    }
    "ok".into()
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

fn astr_from_until_nul(buf: &[u8]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..len]).into_owned()
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
