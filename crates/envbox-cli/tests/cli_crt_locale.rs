//! UCRT empty-locale regression coverage.
//!
//! Python calls the UCRT directly for `locale.setlocale(LC_ALL, "")`.  This
//! test keeps that path separate from the Win32 Probe so a future change to
//! the NLS hooks cannot accidentally hide a UCRT regression.

use std::process::Command;
use uuid::Uuid;

fn envbox_with_root(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_envbox"));
    command.env("ENVBOX_CONFIG_ROOT", root);
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        command.env("ENVBOX_RUNTIME_DLL", dll);
    }
    command
}

fn runtime_dll() -> Option<std::path::PathBuf> {
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        let path = std::path::PathBuf::from(dll);
        if path.is_file() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.parent().map(|p| p.join("envbox-runtime64.dll")),
        Some(dir.join("envbox-runtime64.dll")),
    ]
    .into_iter()
    .flatten()
    .find(|path| path.is_file())
}

fn field(output: &str, name: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .unwrap_or_else(|| panic!("missing {name} in:\n{output}"))
        .trim()
        .to_string()
}

fn probe_exe() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_BIN_EXE_envbox"));
    let dir = dir.parent().expect("envbox.exe parent");
    [
        dir.join("envbox-probe.exe"),
        dir.parent().unwrap().join("envbox-probe.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .expect("envbox-probe.exe required for UCRT acceptance")
}

fn probe_field(output: &str, name: &str) -> String {
    let (_, value) = output
        .split_once(name)
        .unwrap_or_else(|| panic!("missing {name} in:\n{output}"));
    value
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

#[test]
fn bundled_probe_checks_ucrt_with_profile_without_python() {
    let dll = runtime_dll().expect("runtime DLL required for UCRT acceptance");
    let root = std::env::temp_dir().join(format!("envbox-crt-probe-{}", Uuid::new_v4()));
    let profile = envbox_with_root(&root)
        .args([
            "profile",
            "add",
            "--name",
            "CRT Probe US",
            "--locale",
            "en-US",
            "--ui-language",
            "en-US",
            "--region",
            "US",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles",
        ])
        .output()
        .expect("profile add");
    assert!(profile.status.success(), "{profile:?}");
    let profile_id = String::from_utf8_lossy(&profile.stdout).trim().to_string();
    let output = envbox_with_root(&root)
        .env("ENVBOX_RUNTIME_DLL", dll)
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "C")
        .args(["run", "--profile", &profile_id])
        .arg(probe_exe())
        .output()
        .expect("run bundled Probe");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("EnvBox Runtime Loaded"), "{stdout}");
    for name in ["UCRT_setlocale_empty:", "UCRT_wsetlocale_empty:"] {
        assert_eq!(probe_field(&stdout, name), "en-US", "{stdout}");
    }
    assert_eq!(probe_field(&stdout, "LANG:"), "<unset>");
    assert_eq!(probe_field(&stdout, "LC_ALL:"), "<unset>");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ucrt_empty_locale_uses_profile_and_preserves_explicit_calls() {
    let Some(dll) = runtime_dll() else {
        eprintln!("skip: envbox-runtime64.dll not available");
        return;
    };
    let Some(python) = ["python.exe", "python"].into_iter().find(|candidate| {
        Command::new(candidate)
            .args(["-c", "import sys; print(sys.version_info[0])"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }) else {
        eprintln!("skip: Python not available");
        return;
    };

    let root = std::env::temp_dir().join(format!("envbox-crt-locale-{}", Uuid::new_v4()));
    let mut add = envbox_with_root(&root);
    add.env("ENVBOX_RUNTIME_DLL", &dll);
    let profile = add
        .args([
            "profile",
            "add",
            "--name",
            "CRT Locale US",
            "--locale",
            "en-US",
            "--ui-language",
            "en-US",
            "--region",
            "US",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles",
            "--env",
            "LANG=en_US.UTF-8",
        ])
        .output()
        .expect("profile add");
    assert!(profile.status.success(), "{profile:?}");
    let profile_id = String::from_utf8_lossy(&profile.stdout).trim().to_string();

    let mut run = envbox_with_root(&root);
    run.env("ENVBOX_RUNTIME_DLL", &dll);
    let output = run
        .args(["run", "--profile", &profile_id])
        .arg(python)
        .args([
            "-c",
            concat!(
                "import ctypes, locale, os\n",
                "u=ctypes.CDLL('ucrtbase')\n",
                "u.setlocale.argtypes=[ctypes.c_int, ctypes.c_char_p]\n",
                "u.setlocale.restype=ctypes.c_char_p\n",
                "u._wsetlocale.argtypes=[ctypes.c_int, ctypes.c_wchar_p]\n",
                "u._wsetlocale.restype=ctypes.c_wchar_p\n",
                "print('runtime=' + str(os.environ.get('ENVBOX_RUNTIME_LOADED')))\n",
                "print('py=' + locale.setlocale(locale.LC_ALL, ''))\n",
                "print('a=' + str(u.setlocale(0, b'')))\n",
                "print('w=' + str(u._wsetlocale(0, '')))\n",
                "print('q=' + str(u._wsetlocale(0, None)))\n",
                "os.environ['LC_NUMERIC']='C'\n",
                "print('category=' + locale.setlocale(locale.LC_NUMERIC, ''))\n",
                "print('composite=' + locale.setlocale(locale.LC_ALL, ''))\n",
                "print('explicit=' + str(u._wsetlocale(0, 'C')))\n",
            ),
        ])
        .output()
        .expect("run Python UCRT probe");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(field(&stdout, "runtime="), "1");
    assert_eq!(field(&stdout, "py="), "en_US.UTF-8");
    assert_eq!(field(&stdout, "a="), "b'en_US.UTF-8'");
    assert_eq!(field(&stdout, "w="), "en_US.UTF-8");
    assert_eq!(field(&stdout, "q="), "en_US.UTF-8");
    assert_eq!(field(&stdout, "category="), "C");
    assert!(field(&stdout, "composite=").contains("LC_NUMERIC=C"));
    assert_eq!(field(&stdout, "explicit="), "C");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ucrt_empty_locale_without_lang_ignores_conflicting_host_lang() {
    let Some(dll) = runtime_dll() else {
        eprintln!("skip: envbox-runtime64.dll not available");
        return;
    };
    if Command::new("python.exe")
        .args(["-c", "import sys"])
        .output()
        .map(|output| !output.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: Python not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("envbox-crt-no-lang-{}", Uuid::new_v4()));
    let profile = envbox_with_root(&root)
        .args([
            "profile",
            "add",
            "--name",
            "CRT Locale No LANG",
            "--locale",
            "en-US",
            "--ui-language",
            "en-US",
            "--region",
            "US",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles",
        ])
        .output()
        .expect("profile add");
    assert!(profile.status.success(), "{profile:?}");
    let profile_id = String::from_utf8_lossy(&profile.stdout).trim().to_string();

    let output = envbox_with_root(&root)
        .env("ENVBOX_RUNTIME_DLL", dll)
        .env("LANG", "zh_CN.UTF-8")
        .args(["run", "--profile", &profile_id, "python.exe", "-c"])
        .arg(concat!(
            "import locale, os, subprocess\n",
            "print('crt=' + locale.setlocale(locale.LC_ALL, ''))\n",
            "os.environ['LC_ALL']='C'\n",
            "child=subprocess.check_output(['cmd.exe','/d','/c',",
            "'if defined LC_ALL (echo inherited) else (echo unset)'], text=True)\n",
            "print('child_lc_all=' + child.strip())\n",
        ))
        .output()
        .expect("run Python UCRT probe");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(field(&stdout, "crt="), "en-US");
    assert_eq!(field(&stdout, "child_lc_all="), "unset");
    let _ = std::fs::remove_dir_all(&root);
}
