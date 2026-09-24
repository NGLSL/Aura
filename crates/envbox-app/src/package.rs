//! Package Identity + trust classification for launch targets.
//!
//! WindowsApps must not be treated as "any exe under a folder". Classification
//! is by packaging model (Packaged Win32 / UWP AppContainer), not by directory.

use std::path::Path;

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
    #[allow(dead_code)]
    pub fn badge(&self) -> &'static str {
        match self.injection {
            InjectionSupport::Supported => "可注入",
            InjectionSupport::Delayed => "延迟注入",
            InjectionSupport::Unsupported => "不支持注入",
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

    // shell:AppsFolder\<AUMID>!App — UWP / package AUMID activation.
    if lower.starts_with("shell:appsfolder\\") || (lower.contains("shell:appsfolder\\") && p.contains('!')) {
        return uwp("AUMID / AppsFolder 目标");
    }
    if p.contains('!') && lower.contains("microsoft.") && !looks_like_path(p) {
        return uwp("Package AUMID");
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
    fn apps_folder_aumid_is_unsupported() {
        let c = classify_target(
            r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            "",
        );
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
