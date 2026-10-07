//! Navigation guarded by unsaved edits across application, profile, and workspace pages.

use super::{workspaces, EnvBoxApp, PendingNavigation};
use crate::message::{Message, Nav};
use iced::Task;

impl EnvBoxApp {
    pub fn unsaved_dialog_open(&self) -> bool {
        self.pending_navigation.is_some()
    }

    pub(super) fn has_unsaved_edits(&self) -> bool {
        match self.nav {
            Nav::Workspaces => self.workspaces.dirty(),
            Nav::Apps => {
                self.app_edit_mode
                    && (self.app_draft != self.app_saved_draft
                        || (self.app_draft.id.is_none() && !self.app_draft.name.is_empty()))
            }
            Nav::Profiles => {
                self.profile_edit_mode && self.profile_draft != self.profile_saved_draft
            }
            _ => false,
        }
    }

    pub(super) fn request_navigation(&mut self, action: PendingNavigation) -> Task<Message> {
        if self.has_unsaved_edits() {
            self.pending_navigation = Some(action);
            self.unsaved_error = None;
            Task::none()
        } else {
            self.perform_navigation(action)
        }
    }

    pub(super) fn perform_navigation(&mut self, action: PendingNavigation) -> Task<Message> {
        match action {
            PendingNavigation::Workspace(id) => {
                self.workspace_management.selection_changed();
                self.workspaces.select(
                    id,
                    self.profiles
                        .first()
                        .map(|profile| profile.id)
                        .unwrap_or_default(),
                );
                self.workspace_list()
            }
            PendingNavigation::WorkspaceRefresh => {
                self.workspace_management.selection_changed();
                let selected = self.workspaces.selected;
                self.workspaces = workspaces::WorkspaceState::load(
                    &self.store,
                    self.profiles
                        .first()
                        .map(|profile| profile.id)
                        .unwrap_or_default(),
                );
                match self.store.load_profiles() {
                    Ok(doc) => self.profiles = doc.profiles,
                    Err(err) => self.workspaces.error = Some(err.to_string()),
                }
                if let Some(id) = selected.filter(|id| {
                    self.workspaces
                        .document
                        .containers
                        .iter()
                        .any(|value| value.id == *id)
                }) {
                    self.workspaces.select(
                        Some(id),
                        self.profiles
                            .first()
                            .map(|profile| profile.id)
                            .unwrap_or_default(),
                    );
                }
                match self.store.load_applications() {
                    Ok(doc) => self.applications = doc.applications,
                    Err(err) => self.workspaces.error = Some(err.to_string()),
                }
                self.workspace_list()
            }
            PendingNavigation::Nav(nav) => {
                let previous_nav = self.nav;
                self.nav = nav;
                if nav != Nav::Profiles {
                    self.resume_new_app = false;
                }
                if nav == Nav::Instances {
                    self.instance_filter = None;
                }
                if previous_nav != nav && nav == Nav::Profiles && self.profile_draft.id.is_none() {
                    if let Some(id) = self.profiles.first().map(|profile| profile.id) {
                        self.select_profile(id);
                    }
                }
                if previous_nav != nav && nav == Nav::Apps && self.app_draft.id.is_none() {
                    if let Some(id) = self.applications.first().map(|app| app.id) {
                        self.select_app(id);
                    }
                }
                if nav == Nav::Profiles {
                    self.bind_profile_scope();
                    return self.workspace_list();
                }
                if nav == Nav::Workspaces {
                    return self.workspace_list();
                }
                Task::none()
            }
            PendingNavigation::InstancesOf(id) => {
                self.instance_filter = id;
                self.nav = Nav::Instances;
                self.resume_new_app = false;
                Task::none()
            }
            PendingNavigation::App(id) => {
                self.select_app(id);
                Task::none()
            }
            PendingNavigation::Profile(id) => {
                self.select_profile(id);
                self.workspace_list()
            }
            PendingNavigation::NewApp => self.begin_new_app(),
            PendingNavigation::NewProfile => {
                self.begin_new_profile();
                Task::none()
            }
            PendingNavigation::Exit { remember } => self.finish_exit(remember),
        }
    }
}
