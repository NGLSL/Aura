//! Package discovery (V0.3 ticket 46/48).
//!
//! Enumerate installed WindowsApps / MSIX packages for picker/CLI.
//! Launch must still go through AUMID (`LaunchTarget::Packaged`) — never a
//! WindowsApps exe path.

use envbox_core::{LaunchTarget, PackageIdentity};
use std::path::PathBuf;

/// Honest capability / packaging metadata for a discovered package app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageAppInfo {
    pub display_name: String,
    pub aumid: String,
    pub package_full_name: String,
    pub package_family_name: String,
    pub runtime_behavior: String,
    pub trust_level: String,
}

impl PackageAppInfo {
    pub fn identity(&self) -> PackageIdentity {
        PackageIdentity {
            aumid: self.aumid.clone(),
            package_full_name: self.package_full_name.clone(),
            package_family_name: self.package_family_name.clone(),
        }
    }

    /// Injection Support hint from trust/runtime metadata (not a process probe).
    pub fn injection_support_hint(&self) -> &'static str {
        let t = self.trust_level.to_ascii_lowercase();
        let r = self.runtime_behavior.to_ascii_lowercase();
        if t.contains("appcontainer") || r.contains("appcontainer") {
            "unsupported"
        } else if t.contains("microsoft") && t.contains("store") {
            // Store-signed-only often blocks third-party runtime images.
            "delayed"
        } else {
            "supported"
        }
    }
}

/// Parse AUMID `PackageFamilyName!ApplicationId`.
pub fn parse_aumid(aumid: &str) -> Option<(String, String)> {
    let (fam, app) = aumid.split_once('!')?;
    if fam.is_empty() || app.is_empty() {
        return None;
    }
    Some((fam.to_string(), app.to_string()))
}

/// Extract AUMID from `shell:AppsFolder\<AUMID>!App` or a bare AUMID.
pub fn extract_aumid(path: &str) -> Option<String> {
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
    if p.contains('!') && !p.contains('\\') && !p.contains('/') {
        return Some(p.to_string());
    }
    None
}

/// Resolve package identity for an AUMID (WindowsApps folder scan; fail-open).
pub fn resolve_package_identity(aumid: &str) -> PackageIdentity {
    let family = parse_aumid(aumid.trim())
        .map(|(f, _)| f)
        .unwrap_or_else(|| aumid.trim().to_string());
    let package_full_name = find_package_full_name(&family).unwrap_or_else(|| family.clone());
    PackageIdentity {
        aumid: aumid.trim().to_string(),
        package_full_name,
        package_family_name: family,
    }
}

fn find_package_full_name(family: &str) -> Option<String> {
    let program_files = std::env::var("ProgramFiles").ok()?;
    let root = PathBuf::from(program_files).join("WindowsApps");
    let entries = std::fs::read_dir(root).ok()?;
    let family_l = family.to_ascii_lowercase();
    let (name, hash) = family.rsplit_once('_').unwrap_or((family, ""));
    let name_l = name.to_ascii_lowercase();
    let suffix = format!("__{}", hash.to_ascii_lowercase());
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let folder_l = file_name.to_ascii_lowercase();
        if folder_l == family_l
            || (folder_l.starts_with(&format!("{name_l}_")) && folder_l.ends_with(&suffix))
        {
            return Some(file_name);
        }
    }
    None
}

/// User path → LaunchTarget. AUMID forms become `Packaged` (never CreateProcess).
pub fn launch_target_from_user_path(path: &str) -> LaunchTarget {
    if let Some(aumid) = extract_aumid(path) {
        let identity = resolve_package_identity(&aumid);
        return LaunchTarget::Packaged {
            aumid: identity.aumid,
            package_full_name: identity.package_full_name,
            package_family_name: identity.package_family_name,
        };
    }
    LaunchTarget::Executable {
        path: PathBuf::from(path.trim()),
    }
}

/// Re-map legacy Executable entries that actually hold an AUMID path.
pub fn normalize_launch_target(launch: &LaunchTarget) -> LaunchTarget {
    match launch {
        LaunchTarget::Executable { path } => {
            let s = path.to_string_lossy();
            if extract_aumid(&s).is_some() {
                launch_target_from_user_path(&s)
            } else {
                launch.clone()
            }
        }
        other => other.clone(),
    }
}

/// Discover installed packages. Windows: package catalog via
/// `PackageFamilyName` / Appx manifests under `%ProgramFiles%\WindowsApps`
/// is restricted; V0.3 exposes a pure metadata model plus a best-effort
/// enumerator for the current user's package graph.
pub fn discover_packages() -> Vec<PackageAppInfo> {
    #[cfg(windows)]
    {
        windows_discover_packages()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn windows_discover_packages() -> Vec<PackageAppInfo> {
    // Best-effort: enumerate AppX folders the current user can see and parse
    // AppxManifest.xml DisplayName / Application Id. Fail open to empty.
    use std::path::PathBuf;
    let mut out = Vec::new();
    let Ok(program_files) = std::env::var("ProgramFiles") else {
        return out;
    };
    let root = PathBuf::from(program_files).join("WindowsApps");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("AppxManifest.xml");
        if !manifest.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let full_name = entry.file_name().to_string_lossy().to_string();
        // PackageFamilyName is full name minus version/arch/publisher hash tail.
        let family = package_family_from_full_name(&full_name);
        if let Some(info) = parse_manifest_app(&text, &full_name, &family) {
            out.push(info);
        }
    }
    out
}

fn package_family_from_full_name(full_name: &str) -> String {
    // Foo_Bar_1.0.0.0_x64__hash → Foo_Bar_hash (approx; exact identity comes
    // from the package graph when available).
    let parts: Vec<&str> = full_name.split('_').collect();
    if parts.len() >= 5 {
        format!("{}_{}", parts[0], parts[parts.len() - 1])
    } else {
        full_name.to_string()
    }
}

fn parse_manifest_app(text: &str, full_name: &str, family: &str) -> Option<PackageAppInfo> {
    let display = extract_tag_content(text, "DisplayName")
        .or_else(|| extract_attr(text, "DisplayName"))
        .unwrap_or_else(|| full_name.to_string());
    // Prefer Application Id="..." attribute; fall back to Identity Name.
    let app_id = extract_attr_near(text, "Application", "Id")
        .or_else(|| extract_attr_near(text, "Identity", "Name"))
        .or_else(|| extract_tag_content(text, "Application"))?;
    let aumid = format!("{family}!{app_id}");
    let trust = if text.contains("runFullTrust") {
        "fullTrust"
    } else if text.contains("AppContainer") {
        "appContainer"
    } else {
        "unknown"
    };
    Some(PackageAppInfo {
        display_name: display,
        aumid,
        package_full_name: full_name.to_string(),
        package_family_name: family.to_string(),
        runtime_behavior: "windowsApp".into(),
        trust_level: trust.into(),
    })
}

/// Content between <Tag> and </Tag>.
fn extract_tag_content(text: &str, tag: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let open = format!("<{}", tag.to_ascii_lowercase());
    let start = lower.find(&open)?;
    let gt = lower[start..].find('>')? + start;
    // Self-closing or open tag with attrs — content follows '>' unless `/>`.
    if gt > start && lower[gt - 1..gt].starts_with('/') {
        return None;
    }
    let close = format!("</{}>", tag.to_ascii_lowercase());
    let end = lower[gt + 1..].find(&close)? + gt + 1;
    let content = text[gt + 1..end].trim();
    if content.is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

/// Attribute value on the first element named `tag` (any attribute key).
fn extract_attr(text: &str, attr: &str) -> Option<String> {
    let key = format!("{}=\"", attr);
    let pos = text.find(&key)?;
    let rest = &text[pos + key.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Attribute `attr` on element `tag`, e.g. Application Id="App".
fn extract_attr_near(text: &str, tag: &str, attr: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let open = format!("<{}", tag.to_ascii_lowercase());
    let mut search = 0usize;
    let start = loop {
        let hit = lower[search..].find(&open)? + search;
        let after = hit + open.len();
        // Require a real element boundary so `<Application` does not match
        // `<Applications>`.
        let boundary = lower.as_bytes().get(after).copied().unwrap_or(b'x');
        if boundary == b'>' || boundary == b'/' || boundary == b' ' || boundary == b'\t'
            || boundary == b'\n' || boundary == b'\r'
        {
            break hit;
        }
        search = after;
    };
    let gt = lower[start..].find('>')? + start;
    let attrs = &text[start..gt];
    let key = format!("{}=\"", attr);
    let pos = attrs.to_ascii_lowercase().find(&key.to_ascii_lowercase())?;
    let rest = &attrs[pos + key.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aumid_splits_family_and_app() {
        let (f, a) = parse_aumid("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App").unwrap();
        assert_eq!(f, "Microsoft.WindowsTerminal_8wekyb3d8bbwe");
        assert_eq!(a, "App");
        assert!(parse_aumid("nope").is_none());
    }

    #[test]
    fn full_name_to_family_approx() {
        assert_eq!(
            package_family_from_full_name("Foo_Bar_1.0.0.0_x64__abc"),
            "Foo_abc"
        );
    }

    #[test]
    fn manifest_parse_extracts_aumid() {
        let xml = r#"
        <Package>
          <Identity Name="Foo.Bar" />
          <Properties><DisplayName>Foo Bar</DisplayName></Properties>
          <Applications>
            <Application Id="App" Executable="x.exe" EntryPoint="e">
              <uap:VisualElements DisplayName="Foo Bar" />
            </Application>
          </Applications>
        </Package>
        "#;
        let info = parse_manifest_app(xml, "Foo_Bar_1.0_x64__abc", "Foo_abc").unwrap();
        assert_eq!(info.aumid, "Foo_abc!App");
        assert_eq!(info.display_name, "Foo Bar");
        assert_eq!(info.injection_support_hint(), "supported");
    }

    #[test]
    fn appcontainer_hint_is_unsupported() {
        let xml = r#"
        <Package>
          <Application Id="App" />
          <Extensions><desktop6:Extension Executable="x" /></Extensions>
          <Capabilities>AppContainer</Capabilities>
        </Package>
        "#;
        let info = parse_manifest_app(xml, "X_1.0_x64__y", "X_y").unwrap();
        assert_eq!(info.injection_support_hint(), "unsupported");
    }

    #[test]
    fn extract_aumid_from_shell_and_bare() {
        assert_eq!(
            extract_aumid(r"shell:AppsFolder\OpenAi.Codex_2p2nqsd0c76g0!App"),
            Some("OpenAi.Codex_2p2nqsd0c76g0!App".into())
        );
        assert_eq!(
            extract_aumid("OpenAi.Codex_2p2nqsd0c76g0!App"),
            Some("OpenAi.Codex_2p2nqsd0c76g0!App".into())
        );
        assert_eq!(extract_aumid(r"C:\Program Files\Foo\foo.exe"), None);
    }

    #[test]
    fn aumid_path_becomes_packaged_launch_target() {
        let t = launch_target_from_user_path(r"shell:AppsFolder\OpenAi.Codex_2p2nqsd0c76g0!App");
        match t {
            LaunchTarget::Packaged {
                aumid,
                package_family_name,
                ..
            } => {
                assert_eq!(aumid, "OpenAi.Codex_2p2nqsd0c76g0!App");
                assert_eq!(package_family_name, "OpenAi.Codex_2p2nqsd0c76g0");
            }
            other => panic!("expected Packaged, got {other:?}"),
        }
    }

    #[test]
    fn normalize_rewrites_legacy_aumid_executable() {
        let legacy = LaunchTarget::Executable {
            path: r"shell:AppsFolder\OpenAi.Codex_2p2nqsd0c76g0!App".into(),
        };
        match normalize_launch_target(&legacy) {
            LaunchTarget::Packaged { aumid, .. } => {
                assert_eq!(aumid, "OpenAi.Codex_2p2nqsd0c76g0!App")
            }
            other => panic!("expected Packaged, got {other:?}"),
        }
    }
}
