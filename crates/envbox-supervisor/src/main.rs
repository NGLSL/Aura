#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    if let Err(error) = envbox_supervisor::run_default() {
        eprintln!("Supervisor: {error}");
        std::process::exit(1);
    }
}
