//! Shared Chrome launch contract for Launcher and Supervisor.
use crate::session::{SessionError, SessionStartRequest};
use envbox_core::{Application, DetectionSite, LaunchTarget};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use windows::core::PCWSTR;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowThreadProcessId, HWND_MESSAGE,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChromeContext {
    pub directory: PathBuf,
    pub detection: Option<DetectionSite>,
}
fn error(value: impl std::fmt::Display) -> SessionError {
    SessionError::Unsupported(format!("Chrome：{value}；请关闭对应浏览器后重试"))
}
fn data_root() -> Result<PathBuf, SessionError> {
    if let Some(root) = std::env::var_os("ENVBOX_CONFIG_ROOT") {
        return Ok(root.into());
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("com.aura.envbox"))
        .ok_or_else(|| error("无法确定 Aura 用户数据目录"))
}
/// Read branding without loading or executing the browser. Install channels
/// share "Google Chrome" branding, so that name alone cannot imply Stable.
fn product_name(executable: &Path) -> Option<String> {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let path: Vec<u16> = executable
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None) };
    if size == 0 || size > 1024 * 1024 {
        return None;
    }
    // DWORD alignment is required by the Windows version-information buffer.
    let mut data = vec![0u32; (size as usize).div_ceil(4)];
    unsafe { GetFileVersionInfoW(PCWSTR(path.as_ptr()), 0, size, data.as_mut_ptr().cast()) }
        .ok()?;
    let query = |block: &str| -> Option<(*const u8, usize)> {
        let key: Vec<u16> = block.encode_utf16().chain(Some(0)).collect();
        let mut pointer = std::ptr::null_mut();
        let mut length = 0;
        if !unsafe {
            VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(key.as_ptr()),
                &mut pointer,
                &mut length,
            )
        }
        .as_bool()
            || pointer.is_null()
        {
            return None;
        }
        Some((pointer.cast(), length as usize))
    };
    let (translation, bytes) = query(r"\VarFileInfo\Translation")?;
    if bytes < 4 {
        return None;
    }
    // Every translation is a pair of WORDs (language, code page).
    for offset in (0..bytes.saturating_sub(3)).step_by(4) {
        let language = unsafe { std::ptr::read_unaligned(translation.add(offset).cast::<u16>()) };
        let page = unsafe { std::ptr::read_unaligned(translation.add(offset + 2).cast::<u16>()) };
        if let Some((value, length)) = query(&format!(
            r"\StringFileInfo\{language:04x}{page:04x}\ProductName"
        )) {
            if length == 0 || length > 256 {
                continue;
            }
            let value = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), length) };
            return String::from_utf16(value.strip_suffix(&[0]).unwrap_or(value)).ok();
        }
    }
    None
}

/// Windows defaults follow branding and the registered installation layout,
/// not the executable's basename. See Chromium docs/user_data_dir.md#Windows.
fn default_chrome_directory(launch: &LaunchTarget) -> Result<PathBuf, SessionError> {
    let unknown = || {
        SessionError::Unsupported(
        "Chrome：无法可靠确定此浏览器渠道的默认数据目录；请显式设置 --user-data-dir 或开启独立浏览器数据目录".into())
    };
    let LaunchTarget::Executable { path } = launch else {
        return Err(unknown());
    };
    let product = product_name(path).ok_or_else(unknown)?;
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(unknown)?);
    // These build brands have distinct fixed defaults, including portable CfT.
    if product == "Google Chrome for Testing" {
        return Ok(local.join("Google/Chrome for Testing/User Data"));
    }
    if product == "Chromium" {
        return Ok(local.join("Chromium/User Data"));
    }
    if product != "Google Chrome" {
        return Err(unknown());
    }
    let executable = std::fs::canonicalize(path).map_err(|_| unknown())?;
    // Only recognize actual standard installations. A renamed/copied custom
    // chrome.exe must not inherit Stable's default or touch its data root.
    for root in [
        "ProgramW6432",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "LOCALAPPDATA",
    ] {
        let Some(root) = std::env::var_os(root) else {
            continue;
        };
        let Ok(root) = std::fs::canonicalize(root) else {
            continue;
        };
        for channel in ["Chrome", "Chrome Beta", "Chrome Dev", "Chrome SxS"] {
            let installed = root
                .join("Google")
                .join(channel)
                .join("Application")
                .join("chrome.exe");
            if executable
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&installed.as_os_str().to_string_lossy())
            {
                return Ok(local.join("Google").join(channel).join("User Data"));
            }
        }
    }
    Err(unknown())
}

// Chromium trims each argv token before recognizing prefixes or the terminator.
// Keep the stored argv untouched; only its interpretation uses this view.
fn chrome_token(argument: &str) -> &str {
    argument.trim()
}
fn chrome_switch(argument: &str) -> Option<&str> {
    let token = chrome_token(argument);
    token
        .strip_prefix("--")
        .or_else(|| token.strip_prefix('-'))
        .or_else(|| token.strip_prefix('/'))
}
fn chrome_switch_end(arguments: &[String]) -> usize {
    arguments
        .iter()
        .position(|arg| chrome_token(arg) == "--")
        .unwrap_or(arguments.len())
}

/// Effective root arguments. Requests retain original user arguments for stable reuse comparisons.
/// Proxy settings deliberately apply only to the supported Win32 chrome.exe entry.
pub fn proxy_arguments(req: &SessionStartRequest) -> Result<Vec<String>, SessionError> {
    let mut arguments = req.arguments.clone();
    let Some(proxy) = req.profile.as_ref().and_then(|p| p.browser.proxy.as_ref()) else {
        return Ok(arguments);
    };
    proxy
        .validate()
        .map_err(|e| SessionError::InvalidProfile(e.to_string()))?;
    if !supports_proxy(req) {
        return Ok(arguments);
    }
    let switch_end = chrome_switch_end(&arguments);
    if arguments[..switch_end].iter().any(|arg| {
        let switch = chrome_switch(arg).unwrap_or("");
        let name = switch.split('=').next().unwrap_or(switch);
        [
            "proxy-server",
            "proxy-pac-url",
            "proxy-auto-detect",
            "no-proxy-server",
            "proxy-bypass-list",
        ]
        .iter()
        .any(|switch| name.eq_ignore_ascii_case(switch))
    }) {
        return Err(SessionError::Unsupported("Chrome：Profile 代理与应用手填代理参数冲突；请移除 proxy-server、PAC、自动检测、禁用代理或绕过参数".into()));
    }
    arguments.insert(
        switch_end,
        format!("--proxy-server={}", proxy.chrome_server()),
    );
    Ok(arguments)
}

fn supports_proxy(req: &SessionStartRequest) -> bool {
    matches!(&req.launch, LaunchTarget::Executable { path }
        if path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("chrome.exe")))
}
fn has_proxy(req: &SessionStartRequest) -> bool {
    supports_proxy(req)
        && req
            .profile
            .as_ref()
            .is_some_and(|p| p.browser.proxy.is_some())
}

/// Raw CLI keeps its original directory selection; only proxy runs need ownership checks.
pub(crate) fn prepare_session(req: &mut SessionStartRequest) -> Result<(), SessionError> {
    proxy_arguments(req)?;
    if req.chrome.is_none() && has_proxy(req) {
        configure_directory(req, None, None, None)?;
    }
    Ok(())
}

/// Used before either startup path, never by raw CLI commands without Application identity.
pub fn configure(
    app: &Application,
    req: &mut SessionStartRequest,
    root: Option<&Path>,
    detection: Option<DetectionSite>,
) -> Result<(), SessionError> {
    proxy_arguments(req)?;
    if !app.chrome_independent_data && detection.is_none() && !has_proxy(req) {
        return Ok(());
    }
    if !app.supports_chrome_data() || app.id.is_nil() {
        return Err(error("此功能仅支持已保存的 Chrome 可执行应用"));
    }
    configure_directory(
        req,
        app.chrome_independent_data.then_some(app.id),
        root,
        detection,
    )
}

fn configure_directory(
    req: &mut SessionStartRequest,
    independent_app: Option<Uuid>,
    root: Option<&Path>,
    detection: Option<DetectionSite>,
) -> Result<(), SessionError> {
    let mut explicit = None;
    let switch_end = chrome_switch_end(&req.arguments);
    for (index, arg) in req.arguments[..switch_end].iter().enumerate() {
        let switch = chrome_switch(arg).unwrap_or("");
        if switch.eq_ignore_ascii_case("user-data-dir") {
            explicit = Some(
                req.arguments[..switch_end]
                    .get(index + 1)
                    .map(|value| chrome_token(value))
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| error("user-data-dir 缺少目录"))?
                    .to_owned(),
            );
        } else if let Some((name, value)) = switch.split_once('=') {
            if name.eq_ignore_ascii_case("user-data-dir") {
                explicit = Some(value.to_string());
            }
        }
    }
    let directory = if let Some(application_id) = independent_app {
        if explicit.is_some() {
            return Err(error(
                "独立数据目录与手填 --user-data-dir 冲突；移除参数或关闭开关",
            ));
        }
        let root = root
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(data_root)?;
        let identity = req
            .profile
            .as_ref()
            .map(|p| p.id.to_string())
            .unwrap_or_else(|| "host".into());
        let directory = root
            .join("browser-data")
            .join(application_id.to_string())
            .join(identity);
        directory
    } else if let Some(value) = explicit {
        if value.is_empty() {
            return Err(error("user-data-dir 不能为空"));
        }
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            crate::launcher::effective_working_directory(
                &req.launch,
                req.working_directory.as_deref(),
            )?
            .unwrap_or(std::env::current_dir().map_err(error)?)
            .join(path)
        }
    } else {
        default_chrome_directory(&req.launch)?
    };
    std::fs::create_dir_all(&directory).map_err(error)?;
    let directory = std::fs::canonicalize(directory).map_err(error)?;
    // Chrome's singleton title uses a non-verbatim absolute path.
    let directory = PathBuf::from(
        directory
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .unwrap_or(&directory.to_string_lossy()),
    );
    if independent_app.is_some() {
        req.arguments.insert(
            switch_end,
            format!("--user-data-dir={}", directory.display()),
        );
    }
    req.working_directory = crate::launcher::effective_working_directory(
        &req.launch,
        req.working_directory.as_deref(),
    )?;
    req.chrome = Some(ChromeContext {
        directory,
        detection,
    });
    Ok(())
}
fn singleton(directory: &Path) -> Option<u32> {
    let class: Vec<u16> = "Chrome_MessageWindow"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let title: Vec<u16> = directory
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let window = unsafe {
        FindWindowExW(
            HWND_MESSAGE,
            None,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
        )
    }
    .ok()?;
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    (pid != 0).then_some(pid)
}
fn creation_time(pid: u32) -> Result<u64, SessionError> {
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    }
    .map_err(error)?;
    let handle = crate::launcher::win::SafeHandle(handle);
    if unsafe { WaitForSingleObject(handle.0, 0) }.0 != 258 {
        return Err(error("浏览器进程已退出"));
    }
    let mut created = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(handle.0, &mut created, &mut exit, &mut kernel, &mut user) }
        .map_err(error)?;
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
fn normalized(req: &SessionStartRequest) -> SessionStartRequest {
    let mut request = req.clone();
    if let Some(chrome) = &mut request.chrome {
        chrome.detection = None;
    }
    request
}
#[derive(Serialize, Deserialize)]
struct Ownership {
    request: SessionStartRequest,
    instance_id: Uuid,
    pid: u32,
    creation_time: u64,
}
pub struct ChromeLease {
    file: std::fs::File,
    request: SessionStartRequest,
}
impl ChromeLease {
    pub fn acquire(req: &SessionStartRequest) -> Result<Option<Self>, SessionError> {
        let Some(context) = &req.chrome else {
            return Ok(None);
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .share_mode(1)
            .open(context.directory.join(".aura-browser-owner.json"))
            .map_err(|_| error("数据目录正在被 Aura 使用，不能创建第二个根进程"))?;
        if singleton(&context.directory).is_some() {
            return Err(error("目录已被无法确认归属的 Chrome 使用"));
        }
        Ok(Some(Self {
            file,
            request: normalized(req),
        }))
    }
    pub fn publish(&mut self, instance_id: Uuid, pid: u32) -> Result<(), SessionError> {
        let context = self.request.chrome.as_ref().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if creation_time(pid).is_err() {
                return Err(error("启动进程已退出，不能把转交到旧窗口视为新实例"));
            }
            if let Some(owner) = singleton(&context.directory) {
                if owner != pid {
                    return Err(error("Chrome 将启动转交到已有窗口，拒绝认定为新实例"));
                }
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(error("未能确认实际 Chrome 根窗口与数据目录"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let record = Ownership {
            request: self.request.clone(),
            instance_id,
            pid,
            creation_time: creation_time(pid)?,
        };
        self.file.set_len(0).map_err(error)?;
        self.file.seek(SeekFrom::Start(0)).map_err(error)?;
        self.file
            .write_all(&serde_json::to_vec(&record).map_err(error)?)
            .map_err(error)?;
        self.file.sync_data().map_err(error)
    }
}
/// Capability created only after reuse_detection verifies the live owner and full request.
/// Its request cannot be replaced by another caller to bypass a directory lease.
pub(crate) struct VerifiedDetectionHandoff {
    request: SessionStartRequest,
}
impl VerifiedDetectionHandoff {
    pub(crate) fn into_request(self) -> SessionStartRequest {
        self.request
    }
}

/// Open a fixed page only when a live owner and the exact effective configuration match.
pub fn reuse_detection(req: &SessionStartRequest) -> Result<Option<Uuid>, SessionError> {
    let Some(context) = &req.chrome else {
        return Ok(None);
    };
    let Some(site) = context.detection else {
        return Ok(None);
    };
    // A successful write-open proves no lifetime lease exists. Stale records are never ownership evidence.
    let path = context.directory.join(".aura-browser-owner.json");
    if let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(1)
        .open(&path)
    {
        drop(file);
        if singleton(&context.directory).is_some() {
            return Err(error("已有 Chrome 不属于可确认的 Aura 会话"));
        }
        return Ok(None);
    }
    if !path.exists() {
        return Ok(None);
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&path)
        .map_err(error)?;
    let mut data = Vec::new();
    Read::by_ref(&mut file)
        .take(1024 * 1024)
        .read_to_end(&mut data)
        .map_err(error)?;
    let record: Ownership =
        serde_json::from_slice(&data).map_err(|_| error("浏览器正在启动或归属记录不可用"))?;
    if record.request != normalized(req)
        || creation_time(record.pid)? != record.creation_time
        || singleton(&context.directory) != Some(record.pid)
    {
        return Err(error("已有 Chrome 使用不同或无法确认的启动配置"));
    }
    if !matches!(req.launch, LaunchTarget::Executable { .. }) {
        return Err(error("不是 Chrome 可执行应用"));
    }
    // Use the existing launch pipeline even for the short singleton handoff.
    // If the old browser exits in the meantime, the replacement cannot escape
    // into an uninjected Host browser; its own Job is terminated on mismatch.
    let mut handoff = req.clone();
    handoff.chrome = None;
    handoff.arguments.push(site.url().into());
    let mut session =
        crate::session::start_detection_handoff(VerifiedDetectionHandoff { request: handoff })?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let exited = session
            .process_handle()
            .is_some_and(|handle| unsafe { WaitForSingleObject(handle, 0) }.0 == 0);
        if exited {
            let success = session.wait_root()? == 0;
            if !success
                || singleton(&context.directory) != Some(record.pid)
                || creation_time(record.pid)? != record.creation_time
            {
                if let Some(job) = &session.job {
                    let _ = job.terminate();
                }
                return Err(error("检测页未转交给原有受控浏览器"));
            }
            break;
        }
        if std::time::Instant::now() >= deadline {
            if let Some(job) = &session.job {
                let _ = job.terminate();
            }
            return Err(error("原浏览器已退出或检测页转交未完成"));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Ok(Some(record.instance_id))
}
