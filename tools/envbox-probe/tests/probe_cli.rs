//! Probe process I/O seam: observe `envbox-probe` stdout, not internals.

use std::process::Command;

fn probe_exe() -> Command {
    // Integration tests run against the built binary via CARGO_BIN_EXE.
    Command::new(env!("CARGO_BIN_EXE_envbox-probe"))
}

#[test]
fn host_snapshot_prints_all_ticket01_sections() {
    let output = probe_exe().output().expect("run probe");
    assert!(output.status.success(), "probe failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for header in [
        "=== GEO ===",
        "=== LOCALE ===",
        "=== LANGUAGE ===",
        "=== TIMEZONE ===",
        "=== DNS ===",
        "=== ENV ===",
    ] {
        assert!(stdout.contains(header), "missing {header} in:\n{stdout}");
    }
    // Independent literal key names from the product spec / Probe format.
    for key in [
        "GetUserDefaultGeoName:",
        "GetUserDefaultLocaleName:",
        "GetUserDefaultUILanguage:",
        "GetDynamicTimeZoneInformation:",
        "GetNetworkParams:",
        "LANG:",
    ] {
        assert!(stdout.contains(key), "missing key {key} in:\n{stdout}");
    }
}

#[test]
fn spawn_child_prints_parent_and_child_blocks() {
    let output = probe_exe()
        .arg("--spawn-child")
        .output()
        .expect("run probe --spawn-child");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("=== PARENT PROBE ==="),
        "missing parent block:\n{stdout}"
    );
    assert!(
        stdout.contains("=== CHILD PROBE ==="),
        "missing child block:\n{stdout}"
    );
}
