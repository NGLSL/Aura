//! envbox-broker.exe — Session Registry + Runtime IPC Bootstrap host.
//!
//! Usage:
//!   envbox-broker [--pipe NAME]
//!
//! Default pipe: `\\.\pipe\envbox-runtime` (override ENVBOX_IPC_PIPE or --pipe).
//! Hosts (CLI/GUI) register sessions with REGISTER_PROFILE / BIND_PID on the
//! same pipe; Runtime connects with HELLO / GET_PROFILE.

use envbox_broker::{new_registry, serve};
use std::sync::atomic::{AtomicBool, Ordering};

fn pipe_name(args: &[String]) -> String {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--pipe" {
            if let Some(v) = args.get(i + 1) {
                return normalize_pipe(v);
            }
        }
        i += 1;
    }
    match std::env::var("ENVBOX_IPC_PIPE") {
        Ok(v) if !v.is_empty() => normalize_pipe(&v),
        _ => envbox_launcher::DEFAULT_PIPE_NAME.to_string(),
    }
}

fn normalize_pipe(v: &str) -> String {
    if v.starts_with(r"\\.\pipe\") {
        v.to_string()
    } else {
        format!(r"\\.\pipe\{v}")
    }
}

static STOP: AtomicBool = AtomicBool::new(false);

fn g_stop() -> bool {
    STOP.load(Ordering::SeqCst)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = pipe_name(&args);
    let registry = new_registry();
    let mut broker = match serve(registry.clone(), name.clone()) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("envbox-broker: failed to bind {name}: {err}");
            std::process::exit(1);
        }
    };
    eprintln!("envbox-broker: listening on {name}");

    #[cfg(windows)]
    install_ctrlc_handler();

    while !g_stop() {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    broker.stop();
    eprintln!("envbox-broker: stopped");
}

#[cfg(windows)]
fn install_ctrlc_handler() {
    use windows::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT};

    unsafe extern "system" fn handler(ctrl: u32) -> windows::Win32::Foundation::BOOL {
        if ctrl == CTRL_C_EVENT {
            STOP.store(true, Ordering::SeqCst);
        }
        windows::Win32::Foundation::TRUE
    }
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(handler), true);
    }
}
