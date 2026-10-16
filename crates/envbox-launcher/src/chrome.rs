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
/// Used before either startup path, never by raw CLI commands without Application identity.
pub fn configure(
    app: &Application,
    req: &mut SessionStartRequest,
    root: Option<&Path>,
    detection: Option<DetectionSite>,
) -> Result<(), SessionError> {
    if !app.chrome_independent_data && detection.is_none() {
        return Ok(());
    }
    if !app.supports_chrome_data() || app.id.is_nil() {
        return Err(error("此功能仅支持已保存的 Chrome 可执行应用"));
    }
    let mut explicit = None;
    for (index, arg) in req.arguments.iter().enumerate() {
        if arg.eq_ignore_ascii_case("--user-data-dir") {
            explicit = Some(
                req.arguments
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| error("user-data-dir 缺少目录"))?
                    .clone(),
            );
        } else if arg.to_ascii_lowercase().starts_with("--user-data-dir=") {
            explicit = Some(arg[16..].to_string());
        }
    }
    let directory = if app.chrome_independent_data {
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
            .join(app.id.to_string())
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
        PathBuf::from(
            std::env::var_os("LOCALAPPDATA").ok_or_else(|| error("无法确定 Chrome 默认目录"))?,
        )
        .join("Google/Chrome/User Data")
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
    if app.chrome_independent_data {
        req.arguments
            .push(format!("--user-data-dir={}", directory.display()));
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
    let mut session = crate::session::start_session(handoff)?;
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
