//! Explicit GitHub release checks and verified installer updates.
//!
//! The GUI calls this module from a worker thread.  No update request is made
//! at startup and this module never installs or replaces the running binary by
//! itself: it downloads the official installer, verifies both release
//! checksums, launches it through the native Windows shell, and lets the GUI
//! decide when it is safe to exit.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
#[cfg(windows)]
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
#[cfg(windows)]
use windows::Win32::UI::Shell::ShellExecuteW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub const REPOSITORY_URL: &str = "https://github.com/NGLSL/Aura";
pub const LATEST_RELEASE_URL: &str = "https://github.com/NGLSL/Aura/releases/latest";

const RELEASE_API_URL: &str = "https://api.github.com/repos/NGLSL/Aura/releases/latest";
const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/NGLSL/Aura/releases/download/";
const INSTALLER_NAME: &str = "aura-setup.exe";
const CHECKSUM_NAME: &str = "aura-setup.exe.sha256";
const CHECK_API_MAX_BYTES: u64 = 4 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_SECONDS: &str = "120";

/// A release asset whose URL and SHA-256 digest were validated against the
/// Aura GitHub release API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumAsset {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerAsset {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    /// The release's `aura-setup.exe.sha256` sidecar.  Automatic installation
    /// requires this asset as an additional release-owned verification step.
    pub checksum: Option<ChecksumAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    UpToDate,
    Available {
        tag: String,
        /// `None` means a newer release exists, but the release does not have
        /// a complete, verifiable official installer pair.  The UI must open
        /// the release page instead of offering automatic installation.
        installer: Option<InstallerAsset>,
    },
}

/// Failure phase for the explicit installer workflow.  Verification errors
/// invalidate the cached file and require a fresh download; launch errors
/// (including a cancelled UAC prompt) leave the verified file available for a
/// retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    Verification(String),
    Launch(String),
}

/// Re-verify a cached installer and then hand it to the native shell.  Keeping
/// the phase distinction here prevents the UI from treating a tampered or
/// deleted cached file as a retryable UAC cancellation.
pub fn verify_and_launch(path: &Path, asset: &InstallerAsset) -> Result<(), InstallError> {
    verify_downloaded_installer(path, asset).map_err(InstallError::Verification)?;
    launch_installer(path).map_err(InstallError::Launch)
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Check the latest stable GitHub release.  This function performs blocking
/// I/O and must be called off the Iced update thread.
pub fn check_latest(current: &str) -> Result<CheckResult, String> {
    let mut command = curl_command()?;
    let output = command
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "5",
            "--max-time",
            "10",
            "--max-filesize",
            &CHECK_API_MAX_BYTES.to_string(),
            "--proto",
            "=https",
            "--tlsv1.2",
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            "User-Agent: Aura",
            "--url",
            RELEASE_API_URL,
        ])
        .output()
        .map_err(|error| format!("无法启动版本检查：{error}"))?;
    if !output.status.success() {
        return Err(curl_failure("GitHub 版本检查失败", &output));
    }
    parse_release_json(&output.stdout, current)
}

fn curl_command() -> Result<Command, String> {
    #[cfg(windows)]
    let mut command = Command::new(system_curl_path()?);
    #[cfg(not(windows))]
    let mut command = Command::new("curl");

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW.0);
    Ok(command)
}

#[cfg(windows)]
fn system_curl_path() -> Result<PathBuf, String> {
    // Do not resolve curl through PATH: a same-named executable in the Aura
    // working directory or a profile-provided PATH could hijack update code.
    // GetSystemDirectoryW also avoids trusting a virtualized SystemRoot value.
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("无法定位 Windows System32 目录".into());
    }
    buffer.truncate(length);
    let directory =
        String::from_utf16(&buffer).map_err(|_| "Windows System32 目录名称无效".to_string())?;
    let path = PathBuf::from(directory).join("curl.exe");
    if !path.is_file() {
        return Err(format!("系统 curl.exe 不存在：{}", path.display()));
    }
    Ok(path)
}

fn parse_release_json(body: &[u8], current: &str) -> Result<CheckResult, String> {
    let release: GithubRelease = serde_json::from_slice(body)
        .map_err(|error| format!("GitHub 版本信息无法解析：{error}"))?;
    let latest = version_parts(&release.tag_name).ok_or("GitHub 版本号无效")?;
    let installed = version_parts(current).ok_or("当前版本号无效")?;
    if latest <= installed {
        return Ok(CheckResult::UpToDate);
    }

    let installer = release
        .assets
        .iter()
        .find(|asset| asset.name == INSTALLER_NAME)
        .and_then(validated_installer_asset);
    let checksum = release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUM_NAME)
        .and_then(validated_checksum_asset);
    let installer = installer.and_then(|installer| {
        checksum.map(|checksum| InstallerAsset {
            url: installer.url,
            size: installer.size,
            sha256: installer.sha256,
            checksum: Some(checksum),
        })
    });

    Ok(CheckResult::Available {
        tag: release.tag_name,
        installer,
    })
}

fn validated_installer_asset(asset: &GithubAsset) -> Option<ValidatedAsset> {
    if asset.size == 0 || !is_release_asset_url(&asset.browser_download_url, INSTALLER_NAME) {
        return None;
    }
    Some(ValidatedAsset {
        url: asset.browser_download_url.clone(),
        size: asset.size,
        sha256: parse_sha256_digest(asset.digest.as_deref())?,
    })
}

fn validated_checksum_asset(asset: &GithubAsset) -> Option<ChecksumAsset> {
    if asset.size == 0 || !is_release_asset_url(&asset.browser_download_url, CHECKSUM_NAME) {
        return None;
    }
    Some(ChecksumAsset {
        url: asset.browser_download_url.clone(),
        size: asset.size,
        sha256: parse_sha256_digest(asset.digest.as_deref())?,
    })
}

struct ValidatedAsset {
    url: String,
    size: u64,
    sha256: String,
}

fn is_release_asset_url(url: &str, asset_name: &str) -> bool {
    url.starts_with(RELEASE_DOWNLOAD_PREFIX)
        && url.ends_with(&format!("/{asset_name}"))
        && !url.contains('?')
        && !url.contains('#')
        && !url.contains('\r')
        && !url.contains('\n')
}

fn parse_sha256_digest(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    let (algorithm, digest) = value.split_once(':')?;
    if !algorithm.eq_ignore_ascii_case("sha256")
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(digest.to_ascii_lowercase())
}

/// Parse the stable `major.minor.patch` version shape used by Aura releases.
/// Prerelease/build suffixes and extra numeric components are deliberately
/// rejected so a malformed tag can never make the UI claim an update.
fn version_parts(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.trim().trim_start_matches(['v', 'V']).split('.');
    let parsed = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(parsed)
}

/// Download and verify the official installer and its release checksum
/// sidecar.  The returned path is a completed `.exe`; partial files are
/// removed on every failure.  The caller owns the path and decides when to
/// launch it.
pub fn download_verified(asset: &InstallerAsset) -> Result<PathBuf, String> {
    let checksum = asset
        .checksum
        .as_ref()
        .ok_or("此版本缺少官方 SHA-256 校验文件，无法自动安装")?;
    if !is_release_asset_url(&asset.url, INSTALLER_NAME)
        || !is_release_asset_url(&checksum.url, CHECKSUM_NAME)
        || asset.size == 0
        || checksum.size == 0
    {
        return Err("更新下载地址或大小未通过安全校验".into());
    }

    let dir = std::env::temp_dir().join("aura-updates");
    fs::create_dir_all(&dir).map_err(|error| format!("无法创建更新目录：{error}"))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("系统时间无效：{error}"))?
        .as_nanos();
    let name = format!("aura-update-{}-{stamp}", std::process::id());
    let pending_installer = dir.join(format!("{name}.exe.part"));
    let pending_checksum = dir.join(format!("{name}.sha256.part"));
    let ready = dir.join(format!("{name}.exe"));

    let result = (|| {
        download_file(&asset.url, &pending_installer, asset.size)?;
        download_file(&checksum.url, &pending_checksum, checksum.size)?;

        let checksum_bytes = fs::read(&pending_checksum)
            .map_err(|error| format!("SHA-256 校验文件无法读取：{error}"))?;
        verify_file(&pending_checksum, checksum.size, &checksum.sha256)?;
        let sidecar_digest =
            parse_checksum_file(&checksum_bytes).ok_or("官方 SHA-256 校验文件格式无效")?;
        if sidecar_digest != asset.sha256 {
            return Err("官方 SHA-256 校验文件与 GitHub API 摘要不一致".into());
        }

        verify_file(&pending_installer, asset.size, &asset.sha256)?;
        fs::rename(&pending_installer, &ready)
            .map_err(|error| format!("无法准备安装包：{error}"))?;
        let _ = fs::remove_file(&pending_checksum);
        Ok(ready.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending_installer);
        let _ = fs::remove_file(&pending_checksum);
        let _ = fs::remove_file(&ready);
    }
    result
}

fn download_file(url: &str, destination: &Path, expected_size: u64) -> Result<(), String> {
    let output = curl_command()?
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            DOWNLOAD_TIMEOUT_SECONDS,
            "--max-filesize",
            &expected_size.to_string(),
            "--proto",
            "=https",
            "--tlsv1.2",
            "--output",
        ])
        .arg(destination)
        .arg("--url")
        .arg(url)
        .output()
        .map_err(|error| format!("无法启动更新下载：{error}"))?;
    if !output.status.success() {
        return Err(curl_failure("更新下载失败", &output));
    }
    let actual_size = fs::metadata(destination)
        .map_err(|error| format!("下载文件大小无法读取：{error}"))?
        .len();
    if actual_size != expected_size {
        return Err(format!(
            "下载文件大小不符：预期 {expected_size}，实际 {actual_size}"
        ));
    }
    Ok(())
}

fn curl_failure(context: &str, output: &std::process::Output) -> String {
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if detail.is_empty() {
        format!("{context} ({})", output.status)
    } else {
        format!("{context} ({}): {detail}", output.status)
    }
}

fn verify_file(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| format!("安装包无法读取：{error}"))?;
    let size = file
        .metadata()
        .map_err(|error| format!("安装包大小无法读取：{error}"))?
        .len();
    if size != expected_size {
        return Err(format!("安装包大小不符：预期 {expected_size}，实际 {size}"));
    }
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("安装包校验失败：{error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        return Err("安装包 SHA-256 校验失败".into());
    }
    Ok(())
}

/// Re-verify a cached installer immediately before launching it.  The
/// installer may have waited while the user handled a profile/app draft, so a
/// successful download alone is not sufficient proof at launch time.
pub fn verify_downloaded_installer(path: &Path, asset: &InstallerAsset) -> Result<(), String> {
    if asset.checksum.is_none() {
        return Err("更新缺少官方 SHA-256 校验信息".into());
    }
    verify_file(path, asset.size, &asset.sha256)
}

fn parse_checksum_file(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    let line = text
        .lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}'))
        .find(|line| !line.is_empty())?;
    let mut fields = line.split_whitespace();
    let digest = parse_sha256_digest(Some(&format!("sha256:{}", fields.next()?)))?;
    let filename = fields.next()?.trim_start_matches('*');
    filename
        .eq_ignore_ascii_case(INSTALLER_NAME)
        .then_some(digest)
}

/// Launch a verified local installer through the native shell.  `runas`
/// gives an NSIS installer the normal UAC path and returns an error when the
/// user cancels elevation, so the GUI only exits after a real launch success.
pub fn launch_installer(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("已校验的安装包不存在".into());
    }

    #[cfg(windows)]
    {
        let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
        let file: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let directory: Vec<u16> = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let code = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR(directory.as_ptr()),
                SW_SHOWNORMAL,
            )
        };
        if code.0 as isize > 32 {
            return Ok(());
        }
        return Err(format!(
            "安装器启动失败（ShellExecute code {}）",
            code.0 as isize
        ));
    }

    #[cfg(not(windows))]
    {
        Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("安装器启动失败：{error}"))
    }
}

/// Open one of Aura's fixed HTTPS pages without invoking a shell command
/// parser.  Keeping the allow-list here prevents a future caller from turning
/// this helper into an arbitrary command or URL launcher.
pub fn open_url(url: &str) -> Result<(), String> {
    if url != REPOSITORY_URL && url != LATEST_RELEASE_URL {
        return Err("不允许打开此链接".into());
    }

    #[cfg(windows)]
    {
        let target: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let code = unsafe {
            ShellExecuteW(
                None,
                PCWSTR::null(),
                PCWSTR(target.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if code.0 as isize > 32 {
            return Ok(());
        }
        return Err(format!(
            "无法打开链接（ShellExecute code {}）",
            code.0 as isize
        ));
    }

    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("无法打开链接：{error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json(include_checksum: bool, digest: &str) -> Vec<u8> {
        let checksum = if include_checksum {
            r#",{"name":"aura-setup.exe.sha256","browser_download_url":"https://github.com/NGLSL/Aura/releases/download/v0.3.3/aura-setup.exe.sha256","size":82,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#
        } else {
            ""
        };
        format!(
            r#"{{"tag_name":"v0.3.3","assets":[{{"name":"aura-setup.exe","browser_download_url":"https://github.com/NGLSL/Aura/releases/download/v0.3.3/aura-setup.exe","size":7,"digest":"{digest}"}}{checksum}]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn compares_numeric_stable_versions() {
        let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(matches!(
            parse_release_json(&release_json(false, digest), "0.3.2"),
            Ok(CheckResult::Available { .. })
        ));
        assert!(matches!(
            parse_release_json(&release_json(false, digest), "v0.3.3"),
            Ok(CheckResult::UpToDate)
        ));
        assert_eq!(version_parts("v0.10.0"), Some([0, 10, 0]));
        assert!(version_parts("0.3.3-beta.1").is_none());
        assert!(version_parts("0.3.3.1").is_none());
    }

    #[test]
    fn requires_verified_installer_and_checksum_pair() {
        let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let result = parse_release_json(&release_json(false, digest), "0.3.2").unwrap();
        assert!(matches!(
            result,
            CheckResult::Available {
                installer: None,
                ..
            }
        ));

        let result = parse_release_json(&release_json(true, digest), "0.3.2").unwrap();
        let CheckResult::Available { installer, .. } = result else {
            panic!("new release expected")
        };
        let installer = installer.expect("verified installer pair");
        assert_eq!(installer.size, 7);
        assert_eq!(installer.sha256, "a".repeat(64));
        assert_eq!(installer.checksum.as_ref().unwrap().size, 82);
    }

    #[test]
    fn rejects_untrusted_asset_urls_and_digests() {
        let body = br#"{"tag_name":"v0.3.3","assets":[{"name":"aura-setup.exe","browser_download_url":"https://example.com/aura-setup.exe","size":7,"digest":"sha256:bad"},{"name":"aura-setup.exe.sha256","browser_download_url":"https://github.com/NGLSL/Aura/releases/download/v0.3.3/aura-setup.exe.sha256","size":82,"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}"#;
        let result = parse_release_json(body, "0.3.2").unwrap();
        assert!(matches!(
            result,
            CheckResult::Available {
                installer: None,
                ..
            }
        ));
    }

    #[test]
    fn parses_release_checksum_formats() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_checksum_file(format!("{digest}  aura-setup.exe\n").as_bytes()),
            Some(digest.clone())
        );
        assert_eq!(
            parse_checksum_file(format!("{digest} *aura-setup.exe\r\n").as_bytes()),
            Some(digest)
        );
        assert!(parse_checksum_file(b"bad  aura-setup.exe").is_none());
        assert!(parse_checksum_file(
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.exe"
        )
        .is_none());
    }

    #[test]
    fn verifies_payload_size_and_digest() {
        let root = std::env::temp_dir().join(format!("aura-updater-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(INSTALLER_NAME);
        fs::write(&path, b"payload").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"payload"));
        assert!(verify_file(&path, 7, &digest).is_ok());
        fs::write(&path, b"corrupt").unwrap();
        assert!(verify_file(&path, 7, &digest).is_err());
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    #[test]
    #[ignore = "需要访问 GitHub 并下载真实安装包；只验证下载，不启动安装器"]
    fn live_release_check_and_download_verified() {
        let CheckResult::Available { installer, tag } = check_latest("0.0.0").unwrap() else {
            panic!("expected a newer release")
        };
        eprintln!("latest Aura release: {tag}");
        let path = download_verified(&installer.expect("release has installer pair")).unwrap();
        assert!(path.is_file());
        let _ = fs::remove_file(path);
    }
}
