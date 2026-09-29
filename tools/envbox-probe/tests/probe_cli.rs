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
        "GetLocaleInfoA_SNAME:",
        "GetUserDefaultUILanguage:",
        "GetDynamicTimeZoneInformation:",
        "WinRT_Calendar_GetTimeZone:",
        "WinRT_Calendar_ChangedTimeZone:",
        "GetSystemTime:",
        "GetLocalTime:",
        "SystemTimeToTzSpecificLocalTime_Now:",
        "SystemTimeToTzSpecificLocalTime_DateBoundary:",
        "HKLM_TimeZone_TimeZoneKeyName_A:",
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

#[test]
fn spawn_as_user_child_reports_child_or_privilege_skip() {
    let output = probe_exe()
        .arg("--spawn-as-user-child")
        .output()
        .expect("run probe --spawn-as-user-child");
    assert!(output.status.success(), "as-user probe failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("=== PARENT PROBE ==="), "{stdout}");
    assert!(stdout.contains("=== CREATEPROCESSASUSERW ==="), "{stdout}");
    if stdout.contains("Status: succeeded") {
        assert!(stdout.contains("=== CHILD PROBE ==="), "{stdout}");
        assert!(stdout.contains("GetUserDefaultLocaleName:"), "{stdout}");
    } else {
        assert!(
            stdout.contains("Status: skipped (") && stdout.contains("Windows privilege error"),
            "unexpected as-user result: {stdout}"
        );
    }
}

/// Ticket 26: --resolve prints a stable RESOLVE section (getaddrinfo seam).
#[test]
fn resolve_flag_prints_getaddrinfo_section() {
    let output = probe_exe()
        .args(["--resolve", "localhost"])
        .output()
        .expect("run probe --resolve");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("=== RESOLVE ==="), "missing RESOLVE:\n{stdout}");
    assert!(stdout.contains("getaddrinfo:"), "missing key:\n{stdout}");
    // localhost must resolve to a loopback address on Host.
    let value = stdout
        .split("getaddrinfo:")
        .nth(1)
        .and_then(|s| s.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim()
        .to_string();
    assert!(
        value.contains("127.0.0.1") || value.contains("::1"),
        "localhost should be loopback, got {value}"
    );
}

/// DnsQueryEx is a separate Windows resolver entry point and must remain
/// visible in the Probe acceptance surface.
#[test]
fn resolve_dnsquery_ex_flag_prints_section() {
    let output = probe_exe()
        .args(["--resolve-dnsquery-ex", "localhost"])
        .output()
        .expect("run probe --resolve-dnsquery-ex");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("=== RESOLVE DNSQUERY EX ==="),
        "missing RESOLVE DNSQUERY EX:\n{stdout}"
    );
    assert!(stdout.contains("DnsQueryEx_A:"), "missing key:\n{stdout}");
    let value = stdout
        .split("DnsQueryEx_A:")
        .nth(1)
        .and_then(|s| s.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim()
        .to_string();
    assert!(
        value.contains("127.0.0.1") || value.contains("::1"),
        "localhost should be loopback, got {value}"
    );
}
