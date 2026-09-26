//! Local installed-app discovery (Start Menu / Desktop), Kite-inspired.
//! Entry collection + `.lnk` resolve + package classification. No UWP COM index yet.

use std::path::{Path, PathBuf};

use crate::package::{Capability, Packaging};

/// One launchable entry found on this machine.
#[derive(Debug, Clone)]
pub struct DiscoveredApp {
    pub name: String,
    pub path: String,
    pub args: String,
    pub work_dir: String,
    /// Shell icon source: `path` or `path,index` (index may be negative).
    pub icon_src: String,
    /// `start-menu` / `desktop`
    pub source: &'static str,
    /// Package Identity / trust classification (not folder-based).
    pub capability: Capability,
    /// Cached PNG produced after scan; `None` until icon extraction.
    pub icon_png: Option<PathBuf>,
}

impl DiscoveredApp {
    #[allow(dead_code)]
    pub fn is_store(&self) -> bool {
        !matches!(self.capability.packaging, Packaging::Win32)
    }

    pub fn icon_key(&self) -> String {
        format!("{}|{}", self.path.to_lowercase(), self.icon_src.to_lowercase())
    }
}

const MAX_ITEMS: usize = 400;
const MAX_DEPTH: usize = 3;

/// Scan Start Menu + Desktop + shell:AppsFolder for launchable entries.
/// Caps and skips junk names so the picker stays responsive.
pub fn scan_installed_apps() -> Vec<DiscoveredApp> {
    let mut out: Vec<DiscoveredApp> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Store / packaged apps first — Start Menu .lnk GetPath is often empty for them.
    collect_apps_folder(&mut seen, &mut out);
    for (root, source) in scan_roots() {
        if out.len() >= MAX_ITEMS {
            break;
        }
        collect_from_dir(&root, source, MAX_DEPTH, &mut seen, &mut out);
    }
    if out.len() < MAX_ITEMS {
        collect_windows_apps_aliases(&mut seen, &mut out);
    }

    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn scan_roots() -> Vec<(PathBuf, &'static str)> {
    let mut roots = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        roots.push((
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu"),
            "start-menu",
        ));
    }
    if let Ok(progdata) = std::env::var("PROGRAMDATA") {
        roots.push((
            PathBuf::from(progdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu"),
            "start-menu",
        ));
    }
    if let Some(d) = dirs_desktop() {
        roots.push((d, "desktop"));
    }
    roots.push((
        PathBuf::from(r"C:\Users\Public\Desktop"),
        "desktop",
    ));
    roots
}

fn dirs_desktop() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("USERPROFILE") {
        let d = PathBuf::from(p).join("Desktop");
        if d.is_dir() {
            return Some(d);
        }
    }
    None
}

fn collect_from_dir(
    root: &Path,
    source: &'static str,
    depth: usize,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<DiscoveredApp>,
) {
    if !root.is_dir() || out.len() >= MAX_ITEMS || depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for entry in rd.flatten() {
        if out.len() >= MAX_ITEMS {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            let name = file_stem_lower(&path);
            if is_skippable_dir(&name) {
                continue;
            }
            collect_from_dir(&path, source, depth - 1, seen, out);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if is_skippable_shortcut(&file_name) {
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        let resolved = if ext == "lnk" {
            resolve_lnk(&path, source)
        } else if ext == "exe" {
            let path_str = path.display().to_string();
            Some(DiscoveredApp {
                name: display_name_from_stem(&file_name),
                capability: crate::package::classify_target(&path_str, ""),
                path: path_str.clone(),
                args: String::new(),
                work_dir: path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                icon_src: path_str,
                source,
                icon_png: None,
            })
        } else {
            None
        };

        let Some(app) = resolved else { continue };
        // Same launch target (exe path) once — start-menu roots win over desktop.
        let key = app.path.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        out.push(app);
    }
}

fn file_stem_lower(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn display_name_from_stem(file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.to_string());
    stem
}

fn is_skippable_dir(name: &str) -> bool {
    matches!(name, "startup" | "games" | "accessories")
}

fn is_skippable_shortcut(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("uninstall")
        || n.starts_with("help")
        || n.ends_with("license.lnk")
        || n.ends_with("eula.lnk")
        || n.ends_with("readme.lnk")
}

/// Parse `.lnk` via IShellLink (same approach as Kite). Fail-open: bad files skipped.
fn resolve_lnk(path: &Path, source: &'static str) -> Option<DiscoveredApp> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        persist.Load(PCWSTR(wide_path.as_ptr()), STGM_READ).ok()?;

        let mut target_buf = [0u16; 32_768];
        let _ = link.GetPath(&mut target_buf, std::ptr::null_mut(), 0);
        let target_raw = wide_to_string(&target_buf);

        let mut args_buf = [0u16; 4096];
        let _ = link.GetArguments(&mut args_buf);
        let args = non_empty(wide_to_string(&args_buf));

        let mut wd_buf = [0u16; 1024];
        let _ = link.GetWorkingDirectory(&mut wd_buf);

        let mut icon_buf = [0u16; 1024];
        let mut icon_idx = 0i32;
        let _ = link.GetIconLocation(&mut icon_buf, &mut icon_idx);
        let icon_raw = non_empty(wide_to_string(&icon_buf));

        // Shell 型快捷方式 GetPath 为空时，用图标路径兜底，避免商店/系统入口被丢掉。
        let target = if !target_raw.trim().is_empty() {
            target_raw
        } else {
            icon_launch_fallback(icon_raw.as_deref())?
        };

        let work_dir = non_empty(wide_to_string(&wd_buf)).unwrap_or_else(|| {
            Path::new(&target)
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        });

        let icon_src = match icon_raw {
            Some(raw) => format!("{raw},{icon_idx}"),
            None => target.clone(),
        };

        Some(DiscoveredApp {
            name: display_name_from_stem(
                &path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
            capability: crate::package::classify_target(&target, ""),
            path: target,
            args: args.unwrap_or_default(),
            work_dir,
            icon_src,
            source,
            icon_png: None,
        })
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().to_string()
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// icon_location 路径可启动时当作 target（Kite 同款兜底）。
fn icon_launch_fallback(icon_raw: Option<&str>) -> Option<String> {
    let raw = icon_raw?;
    let path_part = raw.rsplit_once(',').map(|(p, _)| p).unwrap_or(raw);
    let expanded = expand_env(path_part);
    if expanded.is_empty() {
        return None;
    }
    if Path::new(&expanded).exists() {
        Some(expanded)
    } else {
        None
    }
}

fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.trim().to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        match rest.find('%') {
            Some(end) => {
                let key = &rest[..end];
                if key.is_empty() {
                    out.push('%');
                } else if let Ok(val) = std::env::var(key) {
                    out.push_str(&val);
                } else {
                    out.push('%');
                    out.push_str(key);
                    out.push('%');
                }
                rest = &rest[end + 1..];
            }
            None => {
                out.push('%');
                out.push_str(rest);
                return out;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Enumerate `shell:AppsFolder` (Store / UWP / packaged desktop) — Kite `uwp`.
/// Store apps often only appear here; Start Menu `.lnk` GetPath is empty.
fn collect_apps_folder(
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<DiscoveredApp>,
) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::StructuredStorage::{
        PropVariantClear, PropVariantToStringAlloc,
    };
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY, PSGetPropertyKeyFromName,
    };
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, BHID_PropertyStore, IEnumShellItems, IShellItem,
        SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let folder: IShellItem = match unsafe {
            SHCreateItemFromParsingName(
                PCWSTR(wide("shell:AppsFolder").as_ptr()),
                None::<&IBindCtx>,
            )
        } {
            Ok(f) => f,
            Err(_) => return,
        };
        let enum_items: IEnumShellItems = match unsafe {
            folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems)
        } {
            Ok(e) => e,
            Err(_) => return,
        };

        let pk_aumid = prop_key("System.AppUserModel.ID");
        let pk_logo = prop_key("System.Tile.SmallLogoPath");
        let pk_install = prop_key("System.AppUserModel.PackageInstallPath");

        loop {
            if out.len() >= MAX_ITEMS {
                break;
            }
            let mut fetched = 0u32;
            let mut slot: Option<IShellItem> = None;
            let ok = unsafe {
                enum_items
                    .Next(std::slice::from_mut(&mut slot), Some(&mut fetched))
                    .is_ok()
            };
            if !ok || fetched == 0 {
                break;
            }
            let Some(item) = slot else { continue };

            let name = display_name(&item).unwrap_or_default();
            if name.is_empty() || is_skippable_shortcut(&name) {
                continue;
            }
            let props: Option<IPropertyStore> = unsafe {
                item.BindToHandler(None::<&IBindCtx>, &BHID_PropertyStore)
                    .ok()
            };
            let Some(props) = props else { continue };
            let Some(aumid) = prop_string(&props, &pk_aumid) else {
                continue;
            };
            if aumid.is_empty() {
                continue;
            }
            let key = format!("aumid|{}", aumid.to_lowercase());
            if !seen.insert(key) {
                continue;
            }

            let install = prop_string(&props, &pk_install).unwrap_or_default();
            let logo = prop_string(&props, &pk_logo).unwrap_or_default();
            let icon_src = if !logo.is_empty() && Path::new(&logo).is_file() {
                logo
            } else if !install.is_empty() {
                install.clone()
            } else {
                format!("shell:AppsFolder\\{aumid}")
            };
            let target = format!("shell:AppsFolder\\{aumid}");
            // Prefer PackageInstallPath → AppxManifest so Full Trust desktop
            // packages are not mislabeled AppContainer just for using AUMID.
            let capability = if !install.is_empty() && Path::new(&install).is_dir() {
                crate::package::classify_install_dir(Path::new(&install))
            } else {
                crate::package::classify_target(&target, "")
            };
            out.push(DiscoveredApp {
                name,
                capability,
                path: target,
                args: String::new(),
                work_dir: String::new(),
                icon_src,
                source: "apps-folder",
                icon_png: None,
            });
        }

        unsafe {
            let _ = CoUninitialize();
        }
    }));
    let _ = result;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    fn prop_key(name: &str) -> PROPERTYKEY {
        let mut key = PROPERTYKEY::default();
        let w = wide(name);
        let _ = unsafe { PSGetPropertyKeyFromName(PCWSTR(w.as_ptr()), &mut key) };
        key
    }
    fn display_name(item: &IShellItem) -> Option<String> {
        unsafe {
            let p = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?;
            if p.is_null() {
                return None;
            }
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0.cast()));
            if s.trim().is_empty() {
                None
            } else {
                Some(s.trim().to_string())
            }
        }
    }
    fn prop_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
        unsafe {
            let Ok(mut var) = store.GetValue(key as *const _ as *const _) else {
                return None;
            };
            let out = match PropVariantToStringAlloc(&var) {
                Ok(p) => {
                    if p.is_null() {
                        None
                    } else {
                        let s = p.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(p.0.cast()));
                        Some(s)
                    }
                }
                Err(_) => None,
            };
            let _ = PropVariantClear(&mut var);
            out.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        }
    }
}

/// `%LOCALAPPDATA%\Microsoft\WindowsApps` execution aliases (non-recursive, capped).
fn collect_windows_apps_aliases(
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<DiscoveredApp>,
) {
    let Ok(local) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let root = PathBuf::from(local).join("Microsoft").join("WindowsApps");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in rd.flatten() {
        if out.len() >= MAX_ITEMS {
            return;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "exe" {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let name = display_name_from_stem(&file_name);
        if name.chars().count() > 24 || is_skippable_shortcut(&file_name) {
            continue;
        }
        let path_str = path.display().to_string();
        let key = path_str.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        out.push(DiscoveredApp {
            name,
            capability: crate::package::classify_target(&path_str, ""),
            path: path_str.clone(),
            args: String::new(),
            work_dir: root.display().to_string(),
            icon_src: path_str,
            source: "windows-apps",
            icon_png: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_env_windir() {
        let got = expand_env(r"%windir%\explorer.exe");
        assert!(got.to_lowercase().ends_with("explorer.exe"), "{got}");
    }

    #[test]
    fn icon_fallback_strips_index() {
        let got = icon_launch_fallback(Some(r"%windir%\explorer.exe,0")).expect("explorer");
        assert!(got.to_lowercase().ends_with("explorer.exe"));
        assert!(!got.contains(','));
    }

    #[test]
    #[ignore = "requires AppsFolder entries in an interactive Windows profile"]
    fn scan_includes_apps_folder_entries() {
        let items = scan_installed_apps();
        assert!(
            !items.is_empty(),
            "scan should find at least one local app"
        );
        // Store/packaged apps (ChatGPT etc.) only reliably appear via AppsFolder.
        assert!(
            items.iter().any(|a| a.source == "apps-folder"),
            "AppsFolder should contribute packaged apps; total={}",
            items.len()
        );
    }

    #[test]
    #[ignore = "requires ChatGPT installed in the interactive Windows profile"]
    fn chat_query_matches_chatgpt() {
        let items = scan_installed_apps();
        let hits: Vec<_> = items
            .iter()
            .filter(|a| {
                let n = a.name.to_ascii_lowercase();
                n.contains("chat") || n.contains("gpt") || a.path.contains("OpenAI.Codex")
            })
            .collect();
        assert!(
            !hits.is_empty(),
            "expected ChatGPT / Chat* via AppsFolder; sample names: {:?}",
            items.iter().take(20).map(|a| a.name.as_str()).collect::<Vec<_>>()
        );
        let chatgpt = items.iter().find(|a| a.name.eq_ignore_ascii_case("ChatGPT"));
        assert!(
            chatgpt.is_some(),
            "AppsFolder should expose display name ChatGPT"
        );
        let app = chatgpt.unwrap();
        assert!(app.source == "apps-folder", "source={}", app.source);
        assert!(app.path.contains('!'), "path should be AUMID: {}", app.path);
        // OpenAI.Codex is Packaged Win32 Full Trust (runFullTrust +
        // Windows.FullTrustApplication) — must not be labeled AppContainer.
        assert_eq!(
            app.capability.injection,
            crate::package::InjectionSupport::Delayed,
            "ChatGPT Full Trust should be 延迟注入, got {:?}",
            app.capability
        );
        assert_eq!(app.capability.packaging, crate::package::Packaging::PackagedWin32);
    }
}
