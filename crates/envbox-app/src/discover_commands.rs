//! Discover user-facing CLI entries from command directories.
//! The picker scans when opened, so changes to PATH are picked up on its next open.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_EXPAND_SZ, REG_SZ,
};

use crate::discover::DiscoveredApp;

const MAX_PATH_ROOTS: usize = 128;
const COMMAND_SOURCE: &str = "commands";

pub fn collect_command_apps(
    seen: &mut HashSet<String>,
    out: &mut Vec<DiscoveredApp>,
    max_items: usize,
) {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let program_data = std::env::var_os("PROGRAMDATA").map(PathBuf::from);
    let windows_dir = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(PathBuf::from);
    let current = std::env::var("PATH").unwrap_or_default();
    let machine = registry_path(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let user = registry_path(HKEY_CURRENT_USER, "Environment");
    let roots = command_roots(
        local.as_deref(),
        program_data.as_deref(),
        windows_dir.as_deref(),
        [
            &current,
            machine.as_deref().unwrap_or(""),
            user.as_deref().unwrap_or(""),
        ],
    );
    collect_from_roots(&roots, seen, out, max_items);
}

fn command_roots(
    local: Option<&Path>,
    program_data: Option<&Path>,
    windows_dir: Option<&Path>,
    paths: [&str; 3],
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    // Codex Desktop injects its versioned bin into its own terminals, but the
    // Aura process may have been started before that terminal existed.
    if let Some(local) = local {
        let bin = local.join(r"OpenAI\Codex\bin");
        if let Some(latest) = std::fs::read_dir(bin).ok().and_then(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|dir| dir.join("codex.exe").is_file())
                .max_by_key(|dir| {
                    dir.join("codex.exe")
                        .metadata()
                        .and_then(|metadata| metadata.modified())
                        .ok()
                })
        }) {
            roots.push(latest);
        }
        roots.push(local.join(r"Microsoft\WinGet\Links"));
    }
    if let Some(program_data) = program_data {
        roots.push(program_data.join(r"chocolatey\bin"));
    }

    let windows_key = windows_dir.map(path_key);
    let mut path_count = 0;
    for part in paths {
        for raw in part.split(';') {
            if path_count >= MAX_PATH_ROOTS {
                break;
            }
            let raw = raw.trim().trim_matches('"');
            if raw.is_empty() {
                continue;
            }
            let root = PathBuf::from(expand_env(raw));
            if !root.is_absolute() || root.to_string_lossy().starts_with(r"\\") {
                continue;
            }
            let key = path_key(&root);
            if windows_key.as_ref().is_some_and(|windows| {
                key == *windows || key.starts_with(&format!("{}\\", windows.trim_end_matches('\\')))
            }) {
                continue;
            }
            roots.push(root);
            path_count += 1;
        }
    }

    let mut unique = HashSet::new();
    roots.retain(|root| unique.insert(path_key(root)));
    roots
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn collect_from_roots(
    roots: &[PathBuf],
    seen: &mut HashSet<String>,
    out: &mut Vec<DiscoveredApp>,
    max_items: usize,
) {
    for root in roots {
        if out.len() >= max_items {
            break;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort_by_key(|path| path_key(path));
        for path in paths {
            if out.len() >= max_items {
                break;
            }
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(command_display_name) else {
                continue;
            };
            let key = path_key(&path);
            if !seen.insert(key) {
                continue;
            }
            let target = path.to_string_lossy().into_owned();
            out.push(DiscoveredApp {
                name,
                path: target.clone(),
                args: String::new(),
                work_dir: root.to_string_lossy().into_owned(),
                icon_src: target.clone(),
                source: COMMAND_SOURCE,
                capability: crate::package::classify_target(&target, ""),
                icon_png: None,
            });
        }
    }
}

fn command_display_name(file_name: &OsStr) -> Option<String> {
    let path = Path::new(file_name);
    let ext = path.extension()?.to_string_lossy();
    // .lnk targets need ShellExecute; EnvBox's Command backend launches PE or
    // cmd/bat via ComSpec, so do not offer a shortcut it cannot run.
    if !["exe", "com", "cmd", "bat"]
        .iter()
        .any(|allowed| ext.eq_ignore_ascii_case(allowed))
    {
        return None;
    }
    let name = path.file_stem()?.to_string_lossy();
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 24 {
        return None;
    }
    let compact = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if [
        "uninstall",
        "unins",
        "updater",
        "setup",
        "installer",
        "repair",
        "crashpad",
        "crashreport",
        "helper",
        "elevated",
        "redist",
        "webview",
        "mcphost",
        "mcpserver",
        "adminserver",
        "packaging",
        "pcappce",
    ]
    .iter()
    .any(|marker| compact.contains(marker))
    {
        return None;
    }
    Some(name.to_string())
}

fn registry_path(hive: HKEY, subkey: &str) -> Option<String> {
    let wide: Vec<u16> = OsStr::new(subkey)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(hive, PCWSTR(wide.as_ptr()), 0, KEY_READ, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let mut result = None;
        let mut index = 0u32;
        loop {
            let mut name = [0u16; 512];
            let mut name_len = name.len() as u32;
            let mut ty = 0u32;
            let mut data = vec![0u8; 4096];
            let mut data_len = data.len() as u32;
            let mut status = RegEnumValueW(
                key,
                index,
                PWSTR(name.as_mut_ptr()),
                &mut name_len,
                None,
                Some(&mut ty),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            );
            if status == ERROR_MORE_DATA {
                data.resize(data_len as usize, 0);
                name_len = name.len() as u32;
                status = RegEnumValueW(
                    key,
                    index,
                    PWSTR(name.as_mut_ptr()),
                    &mut name_len,
                    None,
                    Some(&mut ty),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                );
            }
            if status != ERROR_SUCCESS {
                break;
            }
            if String::from_utf16_lossy(&name[..name_len as usize]).eq_ignore_ascii_case("PATH")
                && (ty == REG_SZ.0 || ty == REG_EXPAND_SZ.0)
            {
                let units: Vec<u16> = data[..data_len as usize]
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .take_while(|unit| *unit != 0)
                    .collect();
                result = Some(String::from_utf16_lossy(&units));
                break;
            }
            index += 1;
        }
        let _ = RegCloseKey(key);
        result
    }
}

fn expand_env(value: &str) -> String {
    if !value.contains('%') {
        return value.to_string();
    }
    let wide: Vec<u16> = OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let need = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), None);
        if need == 0 {
            return value.to_string();
        }
        let mut buffer = vec![0u16; need as usize];
        let read = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buffer));
        if read == 0 || read > need {
            return value.to_string();
        }
        String::from_utf16_lossy(&buffer[..read as usize - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_and_codex_cli_are_discoverable_without_system_commands() {
        let root = std::env::temp_dir().join(format!("envbox-commands-{}", uuid::Uuid::new_v4()));
        let grok_bin = root.join("grok-bin");
        let local = root.join("Local");
        let codex_version = local.join(r"OpenAI\Codex\bin").join("version-a");
        let windows = root.join("Windows");
        std::fs::create_dir_all(&grok_bin).unwrap();
        std::fs::create_dir_all(&codex_version).unwrap();
        std::fs::create_dir_all(windows.join("System32")).unwrap();
        std::fs::write(grok_bin.join("grok.exe"), b"fixture").unwrap();
        std::fs::write(codex_version.join("codex.exe"), b"fixture").unwrap();
        std::fs::write(windows.join("System32").join("cmd.exe"), b"fixture").unwrap();
        std::fs::write(grok_bin.join("grok-helper.exe"), b"fixture").unwrap();
        let path = format!(
            "{};{};relative",
            grok_bin.display(),
            windows.join("System32").display()
        );
        let roots = command_roots(Some(&local), None, Some(&windows), [&path, "", ""]);
        let mut seen = HashSet::new();
        let mut apps = Vec::new();
        collect_from_roots(&roots, &mut seen, &mut apps, 400);
        let names: HashSet<_> = apps.iter().map(|app| app.name.as_str()).collect();
        assert!(names.contains("grok"));
        assert!(names.contains("codex"));
        assert!(!names.contains("cmd"));
        assert!(!names.contains("grok-helper"));
        assert!(apps.iter().all(|app| app.source == COMMAND_SOURCE));
        let grok = apps.iter().find(|app| app.name == "grok").unwrap();
        assert!(!grok.matches_picker_query(""));
        assert!(grok.matches_picker_query("grok"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn wrapper_is_kept_but_unlaunchable_shortcut_is_not() {
        assert_eq!(
            command_display_name(OsStr::new("grok.CMD")),
            Some("grok".into())
        );
        assert_eq!(command_display_name(OsStr::new("short.lnk")), None);
        assert_eq!(command_display_name(OsStr::new("library.dll")), None);
    }

    #[test]
    #[ignore = "requires Grok and Codex Desktop installed on this Windows host"]
    fn installed_grok_and_codex_are_found_by_the_picker_scanner() {
        let apps = crate::discover::scan_installed_apps();
        for name in ["grok", "codex"] {
            assert!(
                apps.iter().any(|app| {
                    app.source == COMMAND_SOURCE && app.name.eq_ignore_ascii_case(name)
                }),
                "{name} CLI was not discovered; scanned {} commands",
                apps.iter()
                    .filter(|app| app.source == COMMAND_SOURCE)
                    .count()
            );
        }
    }
}
