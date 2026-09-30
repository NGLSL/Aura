//! Window close preference and tray transitions.

use iced::Task;
use crate::close_behavior::{self, CloseBehavior};
use crate::message::{Message, StatusKind};
#[cfg(windows)]
use crate::tray::TrayState;
use super::{EnvBoxApp, PendingNavigation};

impl EnvBoxApp {
    pub(super) fn finish_exit(&mut self, remember: bool) -> Task<Message> {
        if remember {
            if let Err(err) = close_behavior::save(self.store.root(), CloseBehavior::Exit) {
                self.set_status(StatusKind::Error, format!("记住关闭选择失败：{err}"));
                return Task::none();
            }
            self.close_behavior = CloseBehavior::Exit;
        }
        iced::exit()
    }

    pub(super) fn request_exit(&mut self) -> Task<Message> {
        let remember = self.close_dialog && self.remember_close_choice;
        self.close_dialog = false;
        self.remember_close_choice = false;
        self.request_navigation(PendingNavigation::Exit { remember })
    }

    pub(super) fn request_close(&mut self) -> Task<Message> {
        if self.update_checking {
            self.set_status(
                StatusKind::Info,
                "更新正在进行，请等待检查或下载完成后再关闭 Aura",
            );
            return Task::none();
        }
        if self.close_dialog || self.tray.is_some() || self.pending_navigation.is_some() {
            return Task::none();
        }
        match self.close_behavior {
            CloseBehavior::Ask => {
                self.close_dialog = true;
                self.remember_close_choice = false;
                self.close_error = None;
                Task::none()
            }
            CloseBehavior::Tray => self.hide_to_tray(),
            CloseBehavior::Exit => self.request_exit(),
        }
    }

    fn save_close_choice(&mut self, choice: CloseBehavior) -> bool {
        if !self.close_dialog || !self.remember_close_choice {
            return true;
        }
        match close_behavior::save(self.store.root(), choice) {
            Ok(()) => {
                self.close_behavior = choice;
                true
            }
            Err(err) => {
                self.close_error = Some(format!("记住选择失败：{err}"));
                false
            }
        }
    }

    pub(super) fn hide_to_tray(&mut self) -> Task<Message> {
        #[cfg(windows)]
        {
            if self.tray.is_some() {
                return Task::none();
            }
            let tray = match TrayState::new() {
                Ok(tray) => tray,
                Err(err) => {
                    self.close_dialog = true;
                    self.close_error = Some(format!("无法最小化到系统托盘：{err}"));
                    return Task::none();
                }
            };
            if !self.save_close_choice(CloseBehavior::Tray) {
                return Task::none();
            }
            self.tray = Some(tray);
            self.close_dialog = false;
            self.remember_close_choice = false;
            self.close_error = None;
            return iced::window::get_latest().then(|id| {
                id.map(|id| iced::window::change_mode(id, iced::window::Mode::Hidden))
                    .unwrap_or_else(Task::none)
            });
        }
        #[cfg(not(windows))]
        {
            self.close_error = Some("当前平台不支持系统托盘".into());
            Task::none()
        }
    }

}
