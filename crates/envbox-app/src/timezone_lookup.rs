//! Explicit IP-specific timezone lookup. No country/offset fallback or retries.
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::process::Stdio;
#[cfg(not(windows))]
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Current,
    Explicit,
}
impl Source {
    pub const ALL: [Self; 2] = [Self::Current, Self::Explicit];
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "当前网络出口",
            Self::Explicit => "指定出口 IP",
        }
    }
}
impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone)]
pub struct LookupResult {
    pub source: Source,
    pub queried_at: String,
    pub ip: IpAddr,
    pub location: String,
    pub iana: String,
    pub windows: Option<String>,
    pub can_fill: bool,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct LookupState {
    pub source: Source,
    pub input: String,
    pub generation: u64,
    pub pending: bool,
    pub result: Option<LookupResult>,
    pub error: Option<String>,
}
impl LookupState {
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = false;
        self.result = None;
        self.error = None;
    }
}

pub fn parse_public_ip(value: &str) -> Result<IpAddr, String> {
    let ip: IpAddr = value
        .trim()
        .parse()
        .map_err(|_| "请输入公网 IPv4 或 IPv6 地址，不要包含网址、域名或端口".to_string())?;
    let public = match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    };
    if !public {
        return Err("只支持公网 IP；私有、回环、链路本地、组播及保留地址不能查询".into());
    }
    Ok(ip)
}
fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && ((c == 0 && !matches!(ip.octets()[3], 9 | 10)) || c == 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}
fn public_v6(ip: Ipv6Addr) -> bool {
    // IANA globally-reachable unicast, including the more-specific public
    // allocations inside 2001::/23. Never send local/mapped/multicast addresses.
    // https://www.iana.org/assignments/iana-ipv6-special-registry/
    let s = ip.segments();
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return true;
    }
    if (s[0] & 0xe000) != 0x2000 {
        return false;
    }
    if s[0] == 0x2001 && s[1] < 0x200 {
        return (s[1] == 1 && s[2..7] == [0, 0, 0, 0, 0] && (1..=3).contains(&s[7]))
            || s[1] == 3
            || (s[1] == 4 && s[2] == 0x112)
            || (0x20..=0x3f).contains(&s[1]);
    }
    !(s[0] == 0x2001 && s[1] == 0xdb8) && s[0] != 0x2002 && !(s[0] == 0x3fff && s[1] < 0x1000)
}

pub fn query(source: Source, explicit: Option<IpAddr>) -> Result<LookupResult, String> {
    let url = match (source, explicit) {
        (Source::Current, _) => "https://my.ippure.com/v1/info".to_owned(),
        (Source::Explicit, Some(ip)) => format!("https://ip.net.coffee/api/iprisk/{ip}"),
        _ => return Err("请先输入公网 IP".into()),
    };
    let body = request(&url)?;
    let response: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| "查询失败：服务返回格式已变化（不是有效 JSON）".to_string())?;
    let field = |name: &str| {
        response
            .get(name)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
    };
    let reported = field("ip");
    let ip = match explicit {
        Some(expected) => {
            if !reported.is_empty() && reported.parse::<IpAddr>().ok() != Some(expected) {
                return Err("查询失败：响应 IP 与本次指定出口不一致".into());
            }
            expected
        }
        None => reported
            .parse::<IpAddr>()
            .map_err(|_| "查询失败：响应缺少有效出口 IP".to_string())?,
    };
    let iana = field("timezone").to_owned();
    let mapped = if iana.is_empty() {
        None
    } else {
        envbox_storage::iana_to_windows_for_region(&iana, Some(field("countryCode")))
    };
    let windows = mapped.map(str::to_owned);
    let can_fill = mapped.is_some_and(envbox_storage::windows_id_exists);
    let status = if iana.is_empty() {
        "服务未提供具体时区，请手动选择".into()
    } else if mapped.is_none() {
        "此 IANA 时区没有唯一 CLDR Windows 对应项，请手动选择".into()
    } else if !can_fill {
        "此 Windows 时区在当前系统不可用，请手动选择".into()
    } else {
        "已找到可填入的具体时区".into()
    };
    let location = [field("country"), field("region"), field("city")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let queried_at = query_time();
    Ok(LookupResult {
        source,
        queried_at,
        ip,
        location,
        iana,
        windows,
        can_fill,
        status,
    })
}

fn query_time() -> String {
    #[cfg(windows)]
    {
        // Windows GetSystemTime returns UTC, never the selected Profile zone.
        let t = unsafe { windows::Win32::System::SystemInformation::GetSystemTime() };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        )
    }
    #[cfg(not(windows))]
    {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("Unix {seconds}（UTC）")
    }
}

fn request(url: &str) -> Result<Vec<u8>, String> {
    const LIMIT: u64 = 1024 * 1024;
    let mut child = crate::updater::curl_command()?
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "10",
            "--max-filesize",
            "1048576",
            "--proto",
            "=https",
            "--url",
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "查询失败：无法启动系统网络请求".to_string())?;
    let mut body = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or("查询失败：无法读取响应")?
        .take(LIMIT + 1)
        .read_to_end(&mut body);
    if read.is_err() || body.len() as u64 > LIMIT {
        let _ = child.kill();
        let _ = child.wait();
        return Err("查询失败：响应无法读取或超过 1 MiB".into());
    }
    let status = child
        .wait()
        .map_err(|_| "查询失败：无法读取网络请求状态".to_string())?;
    if !status.success() {
        return Err(match status.code() {
            Some(28) => "查询超时：总时限为 10 秒".into(),
            Some(63) => "查询失败：响应超过 1 MiB".into(),
            Some(22) => "查询失败：服务拒绝请求或没有此 IP 的记录".into(),
            Some(60) | Some(51) => "查询失败：HTTPS 证书校验未通过".into(),
            _ => "查询失败：无法连接服务，请检查网络后重试".into(),
        });
    }
    Ok(body)
}
