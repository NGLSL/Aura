//! Host-side Named Pipe server for Runtime IPC Bootstrap (ticket 40).
//!
//! Serves `\\.\pipe\envbox-runtime` (override `ENVBOX_IPC_PIPE`). Line protocol
//! is defined in `runtime/src/ipc_bootstrap.h` / [`crate::ipc`].

use crate::ipc::{IpcMessage, SessionTable};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Shared host session registry used by the pipe server.
pub type SharedTable = Arc<Mutex<SessionTable>>;

/// Background Named Pipe broker. Dropping stops the accept loop.
pub struct HostBroker {
    table: SharedTable,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pipe_name: String,
}

impl HostBroker {
    /// Start serving on the default (or `ENVBOX_IPC_PIPE`) pipe name.
    pub fn start(table: SharedTable) -> std::io::Result<Self> {
        Self::start_on(table, pipe_name())
    }

    /// Start on an explicit pipe path (per-session names avoid cross-test races).
    pub fn start_on(table: SharedTable, pipe_name: String) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let table2 = table.clone();
        let name2 = pipe_name.clone();
        // Do not return until the accept loop has created its first pipe
        // instance. Runtime DLL initialization happens immediately after the
        // launcher returns from this function; without this handoff the
        // client can spend its first retry interval waiting for a server
        // thread that has not been scheduled yet.
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("envbox-ipc-host".into())
            .spawn(move || serve_loop(table2, stop2, name2, Some(ready_tx)))?;

        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(()) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                stop.store(true, Ordering::SeqCst);
                nudge_pipe(&pipe_name);
                let _ = join.join();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "IPC broker did not become ready",
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                stop.store(true, Ordering::SeqCst);
                nudge_pipe(&pipe_name);
                let _ = join.join();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "IPC broker stopped before becoming ready",
                ));
            }
        }
        Ok(Self {
            table,
            stop,
            join: Some(join),
            pipe_name,
        })
    }

    /// Pipe path clients should use (`ENVBOX_IPC_PIPE`).
    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    pub fn table(&self) -> SharedTable {
        self.table.clone()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Nudge a blocked ConnectNamedPipe by opening the pipe as a client.
        nudge_pipe(&self.pipe_name);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for HostBroker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn pipe_name() -> String {
    match std::env::var("ENVBOX_IPC_PIPE") {
        Ok(v) if v.is_empty() => crate::ipc::DEFAULT_PIPE_NAME.to_string(),
        Ok(v) if v.starts_with(r"\\.\pipe\") => v,
        Ok(v) => format!(r"\\.\pipe\{v}"),
        Err(_) => crate::ipc::DEFAULT_PIPE_NAME.to_string(),
    }
}

/// Per-session pipe path so concurrent Runs do not share one name.
pub fn session_pipe_name(instance_id: &str) -> String {
    format!(r"\\.\pipe\envbox-runtime-{instance_id}")
}

fn nudge_pipe(name: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };

        let wide: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            if let Ok(h) = CreateFileW(
                PCWSTR(wide.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            ) {
                let _ = CloseHandle(h);
            }
        }
    }
}

#[cfg(windows)]
fn serve_loop(
    table: SharedTable,
    stop: Arc<AtomicBool>,
    pipe_name: String,
    mut ready: Option<std::sync::mpsc::SyncSender<()>>,
) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }

    while !stop.load(Ordering::SeqCst) {
        let name: Vec<u16> = std::ffi::OsStr::new(&pipe_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let raw = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                8,
                8192,
                8192,
                0,
                None,
            )
        };
        if raw.is_invalid() {
            // Preserve the existing retry behavior for a transient bind
            // failure. start_on will stop the loop if the first instance does
            // not become available within its bounded readiness window.
            if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        let pipe = OwnedHandle(raw);
        if let Some(tx) = ready.take() {
            let _ = tx.send(());
        }

        match unsafe { ConnectNamedPipe(pipe.0, None) } {
            Ok(()) => {}
            Err(_) => {
                let code = unsafe { GetLastError() };
                if code != ERROR_PIPE_CONNECTED {
                    drop(pipe);
                    continue;
                }
            }
        }

        let _ = serve_connection(pipe.0, &table);
        unsafe {
            let _ = FlushFileBuffers(pipe.0);
            let _ = DisconnectNamedPipe(pipe.0);
        }
        drop(pipe);
    }
}

#[cfg(windows)]
fn serve_connection(
    pipe: windows::Win32::Foundation::HANDLE,
    table: &SharedTable,
) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};

    let mut buf = [0u8; 8192];
    let mut acc = Vec::new();
    loop {
        let mut read = 0u32;
        let ok = unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut read), None) };
        if ok.is_err() || read == 0 {
            return Ok(());
        }
        acc.extend_from_slice(&buf[..read as usize]);
        while let Some(pos) = acc.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = acc.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            if line.is_empty() {
                continue;
            }
            let Ok(msg) = IpcMessage::decode_line(&line) else {
                continue;
            };
            let reply = table.lock().unwrap().handle(&msg);
            if let Some(rep) = reply {
                let mut out = rep.encode_line().into_bytes();
                out.push(b'\n');
                let mut written = 0u32;
                let _ = unsafe { WriteFile(pipe, Some(&out), Some(&mut written), None) };
            }
        }
    }
}

#[cfg(not(windows))]
fn serve_loop(
    _table: SharedTable,
    stop: Arc<AtomicBool>,
    _pipe_name: String,
    ready: Option<std::sync::mpsc::SyncSender<()>>,
) {
    if let Some(tx) = ready {
        let _ = tx.send(());
    }
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(not(windows))]
fn serve_connection(_pipe: (), _table: &SharedTable) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::FakeBroker;

    #[test]
    fn shared_table_starts_and_stops() {
        let table: SharedTable = Arc::new(Mutex::new(SessionTable::new()));
        // On non-Windows this is a no-op loop; on Windows it binds the pipe.
        if let Ok(mut broker) = HostBroker::start(table.clone()) {
            let msg = IpcMessage::Hello {
                pid: 1,
                instance_id: "i".into(),
            };
            let _ = broker.table().lock().unwrap().handle(&msg);
            broker.stop();
        }
    }

    #[test]
    fn fake_broker_still_works() {
        let mut b = FakeBroker::new();
        assert!(b.send(IpcMessage::RuntimeReady { pid: 1 }).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn start_returns_after_pipe_instance_exists() {
        use std::os::windows::ffi::OsStrExt;
        use uuid::Uuid;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };

        let name = session_pipe_name(&Uuid::new_v4().to_string());
        let table: SharedTable = Arc::new(Mutex::new(SessionTable::new()));
        let mut broker = HostBroker::start_on(table, name.clone()).expect("broker ready");
        let wide: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let client = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        }
        .expect("start_on must publish a connectable pipe");
        unsafe {
            let _ = CloseHandle(client);
        }
        broker.stop();
    }
}
