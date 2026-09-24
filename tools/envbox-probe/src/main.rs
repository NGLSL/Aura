//! envbox-probe: Host / Profile environment snapshot for EnvBox acceptance.

use envbox_probe::collect_host_snapshot;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let spawn_child = args.iter().any(|a| a == "--spawn-child");
    let is_child = args.iter().any(|a| a == "--child");

    if is_child {
        print_child();
        return ExitCode::SUCCESS;
    }

    print_parent();
    if spawn_child {
        spawn_child_probe();
    }
    ExitCode::SUCCESS
}

fn print_parent() {
    let snapshot = collect_host_snapshot();
    println!("=== PARENT PROBE ===");
    print!("{}", snapshot.render());
}

fn print_child() {
    let snapshot = collect_host_snapshot();
    println!("=== CHILD PROBE ===");
    print!("{}", snapshot.render());
}

fn spawn_child_probe() {
    let exe = std::env::current_exe().expect("current_exe");
    let output = Command::new(exe)
        .arg("--child")
        .output()
        .expect("failed to spawn child probe");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
}
