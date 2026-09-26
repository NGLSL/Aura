//! Package Identity + trust classification for launch targets.
//!
//! WindowsApps must not be treated as "any exe under a folder". Classification
//! is by packaging model (Packaged Win32 / UWP AppContainer), not by directory.

use std::path::{Path, PathBuf};

/// How the target is packaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packaging {
    /// Classic Win32 desktop process.
    Win32,
    /// MSIX packaged desktop / full-trust / medium IL.
    PackagedWin32,
    /// UWP or other AppContainer process.
    AppContainer,
    /// Packaged, but runtime behavior is not clear yet.
    PackagedUnknown,
}

/// What EnvBox V0.2 is allowed to do with Runtime injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionSupport {
    /// CreateProcess(suspended) → inject → resume.
    Supported,
    /// Must activate first (AUMID); inject after PID is known (race window).
    Delayed,
    /// AppContainer / signature policy — refuse; no silent fallback.
    Unsupported,
}

/// Picker / detail capability snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub packaging: Packaging,
    pub injection: InjectionSupport,
    pub runtime_label: &'static str,
    pub trust_label: &'static str,
    pub reason: &'static str,
}

impl Capability {
    pub fn badge(&self) -> &'static str {
        match self.injection {
            InjectionSupport::Supported => "可使用环境配置",
            InjectionSupport::Delayed => "兼容性有限",
            InjectionSupport::Unsupported => "仅可直接启动",
        }
    }

    /// Copy shown in the GUI. `reason` remains a diagnostic description for
    /// the classifier and should not be rendered as product text.
    pub fn user_explanation(&self) -> &'static str {
        match self.injection {
            InjectionSupport::Supported => "可以使用环境配置启动。",
            InjectionSupport::Delayed => {
                "Aura 会在 Windows 应用启动后尝试加载环境，启动初期的读取可能仍使用系统值。"
            }
            InjectionSupport::Unsupported => {
                "此应用暂不支持环境配置启动；可以选择直接启动。"
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_supported(&self) -> bool {
        self.injection == InjectionSupport::Supported
    }
}

/// Classify a launch path / command line without requiring a live process.
pub fn classify_target(path: &str, args: &str) -> Capability {
    let p = path.trim();
    if p.is_empty() {
        return classic();
    }
    let lower = p.to_ascii_lowercase();

    // shell:AppsFolder\<AUMID>!App / bare AUMID — classify by Package Identity
    // (AppxManifest), never by folder name alone. Full Trust desktop packages
    // are Delayed (post-activation inject); WinRT / AppContainer stay Unsupported.
    if lower.starts_with("shell:appsfolder\\")
        || (lower.contains("shell:appsfolder\\") && p.contains('!'))
        || (p.contains('!') && !looks_like_path(p))
    {
        return classify_aumid_target(p);
    }

    // Absolute path under a WindowsApps install root.
    if is_windows_apps_path(&lower) {
        return classify_packaged_exe(Path::new(p));
    }

    // %LOCALAPPDATA%\Microsoft\WindowsApps\*.exe are often package stubs.
    if lower.contains("\\windowsapps\\") {
        return classify_packaged_exe(Path::new(p));
    }

    // Command lines that only mention WindowsApps are not classified as packaged.
    let _ = args;
    classic()
}

/// Classify `shell:AppsFolder\<AUMID>!App` / bare AUMID via package manifest.
///
/// Resolution is Package Identity based (issue 36 decision table), not a
/// blanket "AppsFolder = AppContainer". Unresolved packages stay Delayed so
/// Full Trust desktop packages are never labeled Unsupported up front.
fn classify_aumid_target(target: &str) -> Capability {
    let Some(aumid) = aumid_from_target(target) else {
        return packaged_unknown();
    };
    if let Some(manifest) = find_manifest_by_aumid(&aumid) {
        return classify_manifest(&manifest);
    }
    Capability {
        packaging: Packaging::PackagedUnknown,
        injection: InjectionSupport::Delayed,
        runtime_label: "Packaged",
        trust_label: "Unknown",
        reason: "AUMID 目标，待按 Package Identity 分类",
    }
}

/// Classify an install root (e.g. `System.AppUserModel.PackageInstallPath`).
pub fn classify_install_dir(dir: &Path) -> Capability {
    let manifest = dir.join("AppxManifest.xml");
    if manifest.is_file() {
        return classify_manifest(&manifest);
    }
    packaged_unknown()
}

/// Find `AppxManifest.xml` for `PackageFamilyName!ApplicationId` under WindowsApps.
fn find_manifest_by_aumid(aumid: &str) -> Option<std::path::PathBuf> {
    let family = aumid.split('!').next()?;
    find_manifest_by_family(family)
}

fn find_manifest_by_family(family: &str) -> Option<std::path::PathBuf> {
    let program_files = std::env::var("ProgramFiles").ok()?;
    let root = PathBuf::from(program_files).join("WindowsApps");
    let entries = std::fs::read_dir(&root).ok()?;
    let family_l = family.to_ascii_lowercase();
    // Family is `Name_PublisherHash`; full folder is `Name_Ver_Arch__Hash`.
    let (name, hash) = family.rsplit_once('_').unwrap_or((family, ""));
    let name_l = name.to_ascii_lowercase();
    let suffix = format!("__{}", hash.to_ascii_lowercase());
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let folder_l = file_name.to_ascii_lowercase();
        let matched = folder_l == family_l
            || (folder_l.starts_with(&format!("{name_l}_")) && folder_l.ends_with(&suffix));
        if !matched {
            continue;
        }
        let manifest = entry.path().join("AppxManifest.xml");
        if manifest.is_file() {
            return Some(manifest);
        }
    }
    None
}

fn classic() -> Capability {
    Capability {
        packaging: Packaging::Win32,
        injection: InjectionSupport::Supported,
        runtime_label: "Win32",
        trust_label: "Medium IL",
        reason: "普通桌面进程，CREATE_SUSPENDED 注入",
    }
}

fn uwp(reason: &'static str) -> Capability {
    Capability {
        packaging: Packaging::AppContainer,
        injection: InjectionSupport::Unsupported,
        runtime_label: "Windows App",
        trust_label: "AppContainer",
        reason,
    }
}

fn packaged_win32() -> Capability {
    Capability {
        packaging: Packaging::PackagedWin32,
        injection: InjectionSupport::Delayed,
        runtime_label: "Packaged Win32",
        trust_label: "Full Trust / Medium IL",
        reason: "打包桌面应用：应经 AUMID 激活后注入，存在 race window",
    }
}

fn packaged_unknown() -> Capability {
    Capability {
        packaging: Packaging::PackagedUnknown,
        injection: InjectionSupport::Delayed,
        runtime_label: "Packaged",
        trust_label: "Unknown",
        reason: "已识别为打包应用，运行时模型待探测",
    }
}

fn is_windows_apps_path(lower: &str) -> bool {
    lower.contains("\\windowsapps\\")
}

fn looks_like_path(s: &str) -> bool {
    s.contains('\\') || s.contains('/')
}

/// Prefer AppxManifest `EntryPoint` / `Executable` over directory name.
fn classify_packaged_exe(exe: &Path) -> Capability {
    if let Some(manifest) = find_appx_manifest(exe) {
        return classify_manifest(&manifest);
    }
    // Store install root without a readable manifest: still packaged, not plain Win32.
    packaged_unknown()
}

fn classify_manifest(manifest: &Path) -> Capability {
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return packaged_unknown();
    };
    // Cheap string scan; avoid pulling an XML dependency for a label.
    let t = text.to_ascii_lowercase();
    // Full-trust desktop bridge apps advertise FullTrustApplication.
    if t.contains("fulltrustapplication") {
        return packaged_win32();
    }
    // UWP / WinRT entrypoint.
    if t.contains("windows.winrt")
        || t.contains("entrypoint=\"windows.")
        || t.contains("entrypoint='windows.")
    {
        return uwp("AppxManifest EntryPoint 为 Windows Runtime");
    }
    // Packaged desktop often just has Executable= without a WinRT entrypoint.
    if t.contains("executable=") {
        return packaged_win32();
    }
    packaged_unknown()
}

fn find_appx_manifest(exe: &Path) -> Option<std::path::PathBuf> {
    let mut dir = exe.parent()?;
    for _ in 0..4 {
        let candidate = dir.join("AppxManifest.xml");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

/// Extract `PackageFamilyName!Application` style AUMID from a target string.
#[allow(dead_code)]
pub fn aumid_from_target(path: &str) -> Option<String> {
    let p = path.trim();
    if p.is_empty() {
        return None;
    }
    if let Some(rest) = p
        .strip_prefix("shell:AppsFolder\\")
        .or_else(|| p.strip_prefix("shell:appsfolder\\"))
    {
        if rest.contains('!') {
            return Some(rest.to_string());
        }
    }
    if p.contains('!') && !looks_like_path(p) {
        return Some(p.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_path_is_supported() {
        let c = classify_target(r"C:\Program Files\Foo\foo.exe", "");
        assert_eq!(c.packaging, Packaging::Win32);
        assert_eq!(c.injection, InjectionSupport::Supported);
    }

    #[test]
    fn apps_folder_aumid_is_not_blanket_unsupported() {
        // Unresolved AUMID must not claim AppContainer — Full Trust desktop
        // packages use the same AUMID form (issue 36 decision table).
        let c = classify_target(
            r"shell:AppsFolder\Not.A.Real_Package_abc123xyz!App",
            "",
        );
        assert_eq!(c.packaging, Packaging::PackagedUnknown);
        assert_eq!(c.injection, InjectionSupport::Delayed);
    }

    #[test]
    fn full_trust_manifest_is_delayed() {
        let dir = std::env::temp_dir().join("envbox_pkg_fulltrust_test");
        let _ = std::fs::create_dir_all(&dir);
        let xml = r#"
        <Package>
          <Capabilities><rescap:Capability Name="runFullTrust" /></Capabilities>
          <Application Id="App" Executable="app/ChatGPT.exe" EntryPoint="Windows.FullTrustApplication" />
        </Package>
        "#;
        std::fs::write(dir.join("AppxManifest.xml"), xml).unwrap();
        let c = classify_install_dir(&dir);
        assert_eq!(c.packaging, Packaging::PackagedWin32);
        assert_eq!(c.injection, InjectionSupport::Delayed);
    }

    #[test]
    fn winrt_manifest_is_unsupported() {
        let dir = std::env::temp_dir().join("envbox_pkg_winrt_test");
        let _ = std::fs::create_dir_all(&dir);
        let xml = r#"
        <Package>
          <Application Id="App" Executable="x.exe" EntryPoint="Windows.Application" />
        </Package>
        "#;
        std::fs::write(dir.join("AppxManifest.xml"), xml).unwrap();
        let c = classify_install_dir(&dir);
        assert_eq!(c.packaging, Packaging::AppContainer);
        assert_eq!(c.injection, InjectionSupport::Unsupported);
    }

    #[test]
    fn windows_apps_without_manifest_is_delayed() {
        let c = classify_target(
            r"C:\No\Such\WindowsApps\Example_1.0.0.0_x64__abc123\app\Example.exe",
            "",
        );
        assert_eq!(c.packaging, Packaging::PackagedUnknown);
        assert_eq!(c.injection, InjectionSupport::Delayed);
    }

    #[test]
    fn aumid_parse() {
        assert_eq!(
            aumid_from_target(r"shell:AppsFolder\Foo_bar!App"),
            Some("Foo_bar!App".into())
        );
    }
}
