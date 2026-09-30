//! GUI orchestration for the explicit GitHub update workflow.

use iced::Task;
use std::path::PathBuf;

use crate::message::{Message, StatusKind};
use crate::updater::{self, CheckResult};
use crate::version::PRODUCT_VERSION;

use super::EnvBoxApp;

impl EnvBoxApp {
    pub(super) fn start_update_check(&mut self) -> Task<Message> {
        if self.update_checking {
            return Task::none();
        }
        self.remove_pending_installer();
        self.update_checking = true;
        self.update_status = None;
        self.update_asset = None;
        self.set_status(StatusKind::Info, "正在检查 GitHub 最新版本…");
        Task::perform(
            worker_task(|| updater::check_latest(PRODUCT_VERSION)),
            Message::UpdateResult,
        )
    }

    pub(super) fn finish_update_check(
        &mut self,
        result: Result<CheckResult, String>,
    ) -> Task<Message> {
        self.update_checking = false;
        self.update_installer_path = None;
        self.update_status = Some(result.clone());
        self.update_asset = match &result {
            Ok(CheckResult::Available { installer, .. }) => installer.clone(),
            _ => None,
        };
        match &result {
            Ok(CheckResult::UpToDate) => {
                self.set_status(StatusKind::Success, "当前已经是最新版本");
            }
            Ok(CheckResult::Available {
                tag,
                installer: Some(_),
            }) => {
                self.set_status(StatusKind::Info, format!("发现新版本 {tag}，可下载并安装"));
            }
            Ok(CheckResult::Available {
                tag,
                installer: None,
            }) => {
                self.set_status(
                    StatusKind::Info,
                    format!("发现新版本 {tag}，但缺少可验证的官方安装包"),
                );
            }
            Err(error) => self.set_status(StatusKind::Error, format!("更新检查失败：{error}")),
        }
        Task::none()
    }

    pub(super) fn start_update_install(&mut self) -> Task<Message> {
        if self.update_checking {
            return Task::none();
        }
        if self.has_unsaved_edits() {
            self.set_status(
                StatusKind::Error,
                "请先保存或放弃未保存修改，再安装 Aura 更新",
            );
            return Task::none();
        }

        if self.update_installer_path.is_some() {
            return self.start_pending_installer();
        }
        let Some(asset) = self.update_asset.clone() else {
            self.set_status(StatusKind::Error, "没有可验证的更新安装包，请先检查更新");
            return Task::none();
        };

        self.update_checking = true;
        self.set_status(StatusKind::Info, "正在下载并校验官方安装包…");
        Task::perform(
            worker_task(move || updater::download_verified(&asset)),
            Message::UpdateDownloadResult,
        )
    }

    pub(super) fn finish_update_download(
        &mut self,
        result: Result<PathBuf, String>,
    ) -> Task<Message> {
        self.update_checking = false;
        match result {
            Ok(path) => {
                self.update_installer_path = Some(path.clone());
                if self.has_unsaved_edits() {
                    self.set_status(
                        StatusKind::Error,
                        "安装包已校验，但检测到未保存修改；保存或放弃后再点击安装",
                    );
                    Task::none()
                } else {
                    self.start_pending_installer()
                }
            }
            Err(error) => {
                self.set_status(StatusKind::Error, format!("更新下载失败：{error}"));
                Task::none()
            }
        }
    }

    fn start_pending_installer(&mut self) -> Task<Message> {
        if self.has_unsaved_edits() {
            self.set_status(StatusKind::Error, "请先保存或放弃未保存修改，再启动安装器");
            return Task::none();
        }

        let Some(path) = self.update_installer_path.clone() else {
            self.set_status(StatusKind::Error, "已校验的安装包不存在，请重新下载");
            return Task::none();
        };
        let Some(asset) = self.update_asset.clone() else {
            self.set_status(StatusKind::Error, "更新摘要不存在，请重新检查更新");
            return Task::none();
        };
        self.update_checking = true;
        self.set_status(StatusKind::Info, "正在复核并启动官方安装器…");
        Task::perform(
            worker_task(move || updater::verify_and_launch(&path, &asset)),
            Message::UpdateInstallResult,
        )
    }

    pub(super) fn finish_update_install(
        &mut self,
        result: Result<(), updater::InstallError>,
    ) -> Task<Message> {
        self.update_checking = false;
        match result {
            Ok(()) => {
                // Keep the source file alive while NSIS is starting and
                // reading it.  It is cleared from GUI state; a later cleanup
                // pass can remove stale files after the installer exits.
                self.update_installer_path = None;
                // Runtime child processes own their staged DLLs and continue
                // after the GUI exits, which preserves the existing close
                // behavior contract.  Exit only after ShellExecute accepted
                // the verified installer.
                self.set_status(StatusKind::Success, "安装器已启动，Aura 即将退出");
                iced::exit()
            }
            Err(updater::InstallError::Verification(error)) => {
                self.remove_pending_installer();
                self.set_status(
                    StatusKind::Error,
                    format!("安装包复核失败，已清理缓存，请重新下载：{error}"),
                );
                Task::none()
            }
            Err(updater::InstallError::Launch(error)) => {
                self.set_status(
                    StatusKind::Error,
                    format!("安装器启动失败，可重试：{error}"),
                );
                Task::none()
            }
        }
    }

    pub(super) fn open_releases(&mut self) -> Task<Message> {
        if let Err(error) = updater::open_url(updater::LATEST_RELEASE_URL) {
            self.set_status(
                StatusKind::Error,
                format!("打开 GitHub 发布页失败：{error}"),
            );
        }
        Task::none()
    }

    pub(super) fn open_repository(&mut self) -> Task<Message> {
        if let Err(error) = updater::open_url(updater::REPOSITORY_URL) {
            self.set_status(StatusKind::Error, format!("打开 Aura 仓库失败：{error}"));
        }
        Task::none()
    }

    fn remove_pending_installer(&mut self) {
        if let Some(path) = self.update_installer_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Run blocking curl/hash/shell preparation away from Iced's update thread.
fn worker_task<T, F>(work: F) -> impl std::future::Future<Output = T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    async move {
        let (tx, rx) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        rx.await.expect("update worker thread dropped its result")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::ChecksumAsset;

    fn installer_asset() -> updater::InstallerAsset {
        updater::InstallerAsset {
            url: "https://github.com/NGLSL/Aura/releases/download/v0.3.3/aura-setup.exe".into(),
            size: 7,
            sha256: "a".repeat(64),
            checksum: Some(ChecksumAsset {
                url: "https://github.com/NGLSL/Aura/releases/download/v0.3.3/aura-setup.exe.sha256"
                    .into(),
                size: 82,
                sha256: "b".repeat(64),
            }),
        }
    }

    #[test]
    fn successful_check_is_kept_for_settings_view() {
        let (mut app, _) = EnvBoxApp::new();
        let asset = installer_asset();
        let _ = app.finish_update_check(Ok(CheckResult::Available {
            tag: "v0.3.3".into(),
            installer: Some(asset.clone()),
        }));
        assert!(!app.update_checking);
        assert!(matches!(
            app.update_status,
            Some(Ok(CheckResult::Available { .. }))
        ));
        assert_eq!(app.update_asset, Some(asset));
    }

    #[test]
    fn failed_check_is_kept_for_settings_view() {
        let (mut app, _) = EnvBoxApp::new();
        let _ = app.finish_update_check(Err("network timeout".into()));
        assert!(!app.update_checking);
        assert!(matches!(app.update_status, Some(Err(error)) if error == "network timeout"));
        assert!(app.update_asset.is_none());
        assert!(app.status.contains("更新检查失败"));
    }

    #[test]
    fn failed_install_verification_clears_pending_and_allows_redownload() {
        let (mut app, _) = EnvBoxApp::new();
        let path = std::env::temp_dir().join(format!("aura-pending-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"payload").unwrap();
        app.update_asset = Some(installer_asset());
        app.update_installer_path = Some(path.clone());
        app.update_checking = true;
        let _ = app.finish_update_install(Err(updater::InstallError::Verification(
            "SHA-256 mismatch".into(),
        )));
        assert!(!app.update_checking);
        assert!(app.update_installer_path.is_none());
        assert!(app.update_asset.is_some());
        assert!(app.status.contains("复核失败"));
        assert!(!path.exists());
    }

    #[test]
    fn failed_install_launch_keeps_pending_path_and_does_not_exit() {
        let (mut app, _) = EnvBoxApp::new();
        let path = std::env::temp_dir().join(format!("aura-pending-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"payload").unwrap();
        app.update_asset = Some(installer_asset());
        app.update_installer_path = Some(path.clone());
        app.update_checking = true;
        let _ = app.finish_update_install(Err(updater::InstallError::Launch(
            "user cancelled UAC".into(),
        )));
        assert!(!app.update_checking);
        assert_eq!(app.update_installer_path, Some(path.clone()));
        assert!(app.status.contains("启动失败"));
        let _ = std::fs::remove_file(path);
    }
}
