//! Environment profile editing, persistence, and workspace binding.

use super::{dns_editor, identity_editor, EnvBoxApp, ProfileDraft};
use crate::message::{ComboField, DnsChoice, Message, Nav, StatusKind, WebRtcChoice};
use crate::options;
use envbox_core::{
    BrowserPrivacyProfile, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile,
};
use envbox_storage::{validate_profile, windows_id_to_iana};
use iced::Task;
use std::collections::HashMap;
use uuid::Uuid;

impl EnvBoxApp {
    fn blank_profile(&self) -> ProfileDraft {
        let tz = self
            .timezones
            .iter()
            .find(|t| t.eq_ignore_ascii_case("Pacific Standard Time"))
            .cloned()
            .or_else(|| self.timezones.first().cloned())
            .unwrap_or_default();
        let tz_iana = windows_id_to_iana(&tz).unwrap_or_default().to_string();
        ProfileDraft {
            id: None,
            name: String::new(),
            locale: "en-US".into(),
            ui: "en-US".into(),
            region: "US".into(),
            tz,
            tz_iana,
            dns_mode: DnsChoice::Host,
            dns_editor: Default::default(),
            webrtc: WebRtcChoice::Host,
            identity: Default::default(),
            env: String::new(),
        }
    }

    pub(super) fn begin_new_profile(&mut self) {
        self.workspace_management.selection_changed();
        self.workspaces.clear(Uuid::nil());
        self.nav = Nav::Profiles;
        self.profile_draft = self.blank_profile();
        self.profile_saved_draft = self.profile_draft.clone();
        self.profile_edit_mode = true;
        self.profile_advanced = false;
        self.open_combo = None;
        self.combo_query.clear();
    }

    pub(super) fn select_profile(&mut self, id: Uuid) {
        self.workspace_management.selection_changed();
        if let Some(profile) = self.profiles.iter().find(|profile| profile.id == id) {
            self.profile_draft = profile_to_draft(profile);
            self.profile_saved_draft = self.profile_draft.clone();
            self.profile_edit_mode = false;
            self.profile_advanced = false;
        }
        self.bind_profile_scope();
        self.open_combo = None;
        self.combo_query.clear();
    }

    pub(super) fn bind_profile_scope(&mut self) {
        let previous_scope = self.workspaces.selected;
        let profile = self
            .profile_draft
            .id
            .and_then(|id| self.profiles.iter().find(|profile| profile.id == id))
            .cloned();
        if let Some(profile) = profile {
            if let Err(error) = self.workspaces.bind_profile(&self.store, &profile) {
                self.set_status(StatusKind::Error, format!("环境配置运行准备失败：{error}"));
            }
        } else {
            self.workspaces.clear(Uuid::nil());
        }
        if previous_scope != self.workspaces.selected {
            self.workspace_management.selection_changed();
        }
    }

    /// Options for a searchable select: common-first; query filters; Windows TZ search widens to host list.
    pub fn combo_options(&self, field: ComboField) -> Vec<String> {
        let q = self.combo_query.trim();
        let base = match field {
            ComboField::Region => options::common_regions(),
            ComboField::Locale | ComboField::Ui => options::common_locales(),
            ComboField::Timezone => {
                if q.is_empty() {
                    envbox_storage::common_windows_timezone_ids()
                        .into_iter()
                        .filter(|id| self.timezones.iter().any(|t| t.eq_ignore_ascii_case(id)))
                        .collect()
                } else {
                    self.timezones.clone()
                }
            }
            ComboField::TimezoneIana => envbox_storage::common_iana_timezone_ids(),
        };
        let cur = match field {
            ComboField::Region => self.profile_draft.region.as_str(),
            ComboField::Locale => self.profile_draft.locale.as_str(),
            ComboField::Ui => self.profile_draft.ui.as_str(),
            ComboField::Timezone => self.profile_draft.tz.as_str(),
            ComboField::TimezoneIana => self.profile_draft.tz_iana.as_str(),
        };
        let with_cur = options::with_current(&base, cur);
        let kind = match field {
            ComboField::Region => options::OptionKind::Region,
            ComboField::Locale | ComboField::Ui => options::OptionKind::Locale,
            ComboField::Timezone | ComboField::TimezoneIana => options::OptionKind::Timezone,
        };
        options::filter_options(&with_cur, q, kind)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn open_combo(&mut self, field: ComboField) {
        if self.open_combo == Some(field) {
            self.open_combo = None;
            self.combo_query.clear();
        } else {
            self.open_combo = Some(field);
            self.combo_query.clear();
        }
    }

    pub fn pick_combo(&mut self, field: ComboField, value: String) {
        match field {
            ComboField::Region => self.profile_draft.region = value,
            ComboField::Locale => self.profile_draft.locale = value,
            ComboField::Ui => self.profile_draft.ui = value,
            ComboField::Timezone => {
                self.profile_draft.tz_iana =
                    windows_id_to_iana(&value).unwrap_or_default().to_string();
                self.profile_draft.tz = value;
            }
            ComboField::TimezoneIana => {
                if let Some(win) = envbox_storage::iana_to_windows(&value) {
                    self.profile_draft.tz = win.to_string();
                }
                self.profile_draft.tz_iana = value;
            }
        }
        self.open_combo = None;
        self.combo_query.clear();
    }

    pub fn profile_name(&self, id: Uuid) -> String {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "—".into())
    }

    pub(super) fn save_profile(&mut self) -> Task<Message> {
        let dns = match self
            .profile_draft
            .dns_editor
            .to_profile(self.profile_draft.dns_mode.to_mode())
        {
            Ok(dns) => dns,
            Err(err) => {
                self.set_status(StatusKind::Error, format!("保存失败：{err}"));
                return Task::none();
            }
        };
        let mut environment = HashMap::new();
        for line in self
            .profile_draft
            .env
            .split(|c| c == ';' || c == '\n' || c == '\r')
        {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                self.set_status(
                    StatusKind::Error,
                    format!("保存失败：环境变量需要 KEY=VALUE，收到 {line:?}"),
                );
                return Task::none();
            };
            let k = k.trim();
            if k.is_empty() {
                self.set_status(
                    StatusKind::Error,
                    format!("保存失败：环境变量键为空 {line:?}"),
                );
                return Task::none();
            }
            environment.insert(k.to_string(), v.trim().to_string());
        }
        let profile = EnvironmentProfile {
            id: self.profile_draft.id.unwrap_or_else(Uuid::new_v4),
            name: self.profile_draft.name.clone(),
            locale: LocaleProfile {
                locale_name: self.profile_draft.locale.clone(),
                ui_language: self.profile_draft.ui.clone(),
                region: self.profile_draft.region.clone(),
            },
            timezone: TimezoneProfile {
                windows_id: self.profile_draft.tz.clone(),
                iana_id: self.profile_draft.tz_iana.trim().to_string(),
            },
            dns,
            environment,
            identity: self.profile_draft.identity.to_profile(),
            registry: RegistryProfile::default(),
            browser: BrowserPrivacyProfile {
                webrtc: self.profile_draft.webrtc.to_policy(),
            },
        };
        if let Err(err) = validate_profile(&profile) {
            self.set_status(StatusKind::Error, format!("保存失败: {err}"));
            return Task::none();
        }
        let mut doc = match self.store.load_profiles() {
            Ok(doc) => doc,
            Err(err) => {
                self.set_status(StatusKind::Error, format!("读取配置失败：{err}"));
                return Task::none();
            }
        };
        if let Some(slot) = doc.profiles.iter_mut().find(|p| p.id == profile.id) {
            *slot = profile.clone();
        } else {
            doc.profiles.push(profile.clone());
        }
        match self.store.save_profiles(&doc) {
            Ok(()) => {
                self.profiles = doc.profiles;
                self.profile_draft.id = Some(profile.id);
                self.profile_draft.identity =
                    identity_editor::IdentityDraft::from_profile(&profile.identity);
                self.profile_draft.dns_editor = dns_editor::DnsEditor::from_profile(&profile.dns);
                self.profile_saved_draft = self.profile_draft.clone();
                self.profile_edit_mode = false;
                self.bind_profile_scope();
                if self.workspaces.error.is_none() {
                    self.set_status(StatusKind::Success, "环境配置已保存");
                }
                if self.resume_new_app {
                    self.resume_new_app = false;
                    return self.begin_new_app();
                }
                return self.workspace_list();
            }
            Err(err) => self.set_status(StatusKind::Error, format!("保存失败: {err}")),
        }
        Task::none()
    }

    pub(super) fn delete_profile(&mut self) -> Task<Message> {
        let Some(id) = self.profile_draft.id else {
            self.set_status(StatusKind::Error, "请先选择环境配置");
            return Task::none();
        };
        if self.workspace_management.pending_belongs_to_profile(id) {
            self.set_status(
                StatusKind::Error,
                "此环境配置的启动结果尚未知；请先查询原实例，再删除配置",
            );
            return Task::none();
        }
        let in_use = self
            .applications
            .iter()
            .filter(|app| app.default_profile_id == id)
            .count();
        if in_use > 0 {
            self.set_status(
                StatusKind::Error,
                format!("有 {in_use} 个应用使用此环境配置，请先更改这些应用的默认环境"),
            );
            return Task::none();
        }
        let mut doc = match self.store.load_profiles() {
            Ok(doc) => doc,
            Err(error) => {
                self.set_status(StatusKind::Error, format!("读取配置失败：{error}"));
                return Task::none();
            }
        };
        doc.profiles.retain(|p| p.id != id);
        match self.store.save_profiles(&doc) {
            Ok(()) => {
                self.profiles = doc.profiles;
                if let Some(next_id) = self.profiles.first().map(|profile| profile.id) {
                    self.select_profile(next_id);
                } else {
                    self.profile_draft = self.blank_profile();
                    self.profile_saved_draft = self.profile_draft.clone();
                    self.profile_edit_mode = false;
                    self.workspace_management.selection_changed();
                    self.workspaces.clear(Uuid::nil());
                }
                self.set_status(StatusKind::Success, "环境配置已删除");
            }
            Err(err) => self.set_status(StatusKind::Error, format!("删除失败: {err}")),
        }
        Task::none()
    }
}

pub fn profile_to_draft(p: &EnvironmentProfile) -> ProfileDraft {
    ProfileDraft {
        id: Some(p.id),
        name: p.name.clone(),
        locale: p.locale.locale_name.clone(),
        ui: p.locale.ui_language.clone(),
        region: p.locale.region.clone(),
        tz: p.timezone.windows_id.clone(),
        tz_iana: p.timezone.iana_id.clone(),
        dns_mode: DnsChoice::from_mode(&p.dns.mode),
        dns_editor: dns_editor::DnsEditor::from_profile(&p.dns),
        webrtc: WebRtcChoice::from_policy(&p.browser.webrtc),
        identity: identity_editor::IdentityDraft::from_profile(&p.identity),
        env: p
            .environment
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(";"),
    }
}
