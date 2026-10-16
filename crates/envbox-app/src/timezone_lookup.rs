//! Actual egress timezone lookup through the captured Profile proxy. No country/offset fallback or retries.
use envbox_core::BrowserProxy;
use std::io::Read;
use std::net::IpAddr;
use std::process::Stdio;
#[cfg(not(windows))]
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct LookupResult {
    pub source: String,
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

pub fn query(proxy: Option<&BrowserProxy>) -> Result<LookupResult, String> {
    if let Some(proxy) = proxy {
        proxy.validate().map_err(|e| e.to_string())?;
    }
    let source = proxy
        .map(|p| format!("Profile {}", p.chrome_server()))
        .unwrap_or_else(|| "Aura 当前网络（无 Profile 代理）".into());
    let body = request(proxy)?;
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
    let ip = reported
        .parse::<IpAddr>()
        .map_err(|_| "查询失败：响应缺少有效出口 IP".to_string())?;
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

fn request(proxy: Option<&BrowserProxy>) -> Result<Vec<u8>, String> {
    const LIMIT: u64 = 1024 * 1024;
    let mut command = crate::updater::curl_command()?;
    command.arg("--disable");
    if let Some(proxy) = proxy {
        command.args(["--proxy", &proxy.curl_server(), "--noproxy", ""]);
    }
    let mut child = command
        .args([
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
            "https://my.ippure.com/v1/info",
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
