//! Per-session GUI ownership and second-launch activation.

use std::sync::{Mutex, OnceLock};

use iced::futures::{channel::mpsc, Stream, StreamExt};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, BOOL, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenProcess, QueryFullProcessImageNameW, SetEvent,
    WaitForSingleObject, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, SetForegroundWindow,
    ShowWindow, SW_RESTORE, SW_SHOW,
};

use crate::message::Message;

const NAME: &str = "Local\\Aura.GUI.7d9c7d34-087e-4d41-a3e8-7e9d34fc8d51";

pub enum Claim {
    Primary,
    AlreadyRunning,
}

// Win32 handles remain valid until process exit. Store their pointer values as integers so the
// process-wide owner and the waiting thread can share them without claiming HANDLE is Send/Sync.
struct OwnedHandle(isize);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Self {
        Self(handle.0 as isize)
    }

    fn get(&self) -> HANDLE {
        HANDLE(self.0 as *mut _)
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.get()) };
    }
}

struct Handles {
    event: OwnedHandle,
    _mutex: OwnedHandle,
}

enum NamedClaim {
    Primary(Handles),
    AlreadyRunning,
}

static OWNER: OnceLock<Handles> = OnceLock::new();
static RECEIVER: OnceLock<Mutex<Option<mpsc::UnboundedReceiver<()>>>> = OnceLock::new();

pub fn claim() -> Result<Claim, String> {
    if OWNER.get().is_some() {
        return Ok(Claim::Primary);
    }

    match claim_named(NAME)? {
        NamedClaim::AlreadyRunning => {
            activate_matching_window();
            Ok(Claim::AlreadyRunning)
        }
        NamedClaim::Primary(handles) => {
            let (sender, receiver) = mpsc::unbounded();
            let event = handles.event.0;
            std::thread::Builder::new()
                .name("aura-singleton-activation".into())
                .spawn(move || loop {
                    if unsafe { WaitForSingleObject(HANDLE(event as *mut _), INFINITE) }
                        != WAIT_OBJECT_0
                    {
                        break;
                    }
                    if sender.unbounded_send(()).is_err() {
                        break;
                    }
                })
                .map_err(|error| format!("无法启动 Aura 唤醒监听：{error}"))?;
            RECEIVER
                .set(Mutex::new(Some(receiver)))
                .map_err(|_| "Aura 唤醒通道已初始化".to_string())?;
            OWNER
                .set(handles)
                .map_err(|_| "Aura 单例已初始化".to_string())?;
            Ok(Claim::Primary)
        }
    }
}

fn claim_named(name: &str) -> Result<NamedClaim, String> {
    // Create the auto-reset event first: a second process can signal it even before Iced starts.
    let event_name = wide(&format!("{name}.Event"));
    let event = unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) }
        .map(OwnedHandle::new)
        .map_err(|error| format!("创建 Aura 唤醒事件失败：{error}"))?;

    let mutex_name = wide(&format!("{name}.Mutex"));
    let mutex = unsafe { CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr())) }
        .map(OwnedHandle::new)
        .map_err(|error| format!("创建 Aura 单例锁失败：{error}"))?;
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        unsafe { SetEvent(event.get()) }
            .map_err(|error| format!("通知已运行的 Aura 失败：{error}"))?;
        Ok(NamedClaim::AlreadyRunning)
    } else {
        Ok(NamedClaim::Primary(Handles {
            event,
            _mutex: mutex,
        }))
    }
}

pub fn subscription() -> iced::Subscription<Message> {
    iced::Subscription::run(activation_stream)
}

fn activation_stream() -> impl Stream<Item = Message> {
    RECEIVER
        .get()
        .expect("claim must run before Iced starts")
        .lock()
        .unwrap()
        .take()
        .expect("singleton subscription must start once")
        .map(|()| Message::WindowRestore)
}

fn activate_matching_window() {
    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    let expected = current_exe.to_string_lossy().to_lowercase();
    let mut search = WindowSearch {
        expected,
        found: None,
    };
    let _ = unsafe {
        EnumWindows(
            Some(find_matching_window),
            LPARAM((&mut search as *mut WindowSearch) as isize),
        )
    };
    if let Some(hwnd) = search.found {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

struct WindowSearch {
    expected: String,
    found: Option<HWND>,
}

unsafe extern "system" fn find_matching_window(hwnd: HWND, context: LPARAM) -> BOOL {
    let search = &mut *(context.0 as *mut WindowSearch);
    let mut title = [0u16; 256];
    let length = GetWindowTextW(hwnd, &mut title);
    if length <= 0 || String::from_utf16_lossy(&title[..length as usize]) != "Aura" {
        return BOOL(1);
    }

    let mut process_id = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) else {
        return BOOL(1);
    };
    let process = OwnedHandle::new(process);
    let mut path = [0u16; 32768];
    let mut size = path.len() as u32;
    if QueryFullProcessImageNameW(
        process.get(),
        Default::default(),
        windows::core::PWSTR(path.as_mut_ptr()),
        &mut size,
    )
    .is_ok()
        && String::from_utf16_lossy(&path[..size as usize]).to_lowercase() == search.expected
    {
        search.found = Some(hwnd);
        return BOOL(0);
    }
    BOOL(1)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_claim_signals_then_release_allows_new_owner() {
        let name = format!("Local\\Aura.Singleton.Test.{}", uuid::Uuid::new_v4());
        let NamedClaim::Primary(first) = claim_named(&name).unwrap() else {
            panic!("first claim should own the mutex");
        };
        assert!(matches!(
            claim_named(&name).unwrap(),
            NamedClaim::AlreadyRunning
        ));
        assert_eq!(
            unsafe { WaitForSingleObject(first.event.get(), 0) },
            WAIT_OBJECT_0
        );
        drop(first);
        assert!(matches!(
            claim_named(&name).unwrap(),
            NamedClaim::Primary(_)
        ));
    }
}
