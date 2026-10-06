use super::*;
use envbox_core::Container;
use envbox_storage::ContainerDocument;

pub struct WorkspaceState {
    pub document: ContainerDocument,
    pub draft: Container,
    pub selected: Option<Uuid>,
    pub error: Option<String>,
    saved: Container,
}

impl WorkspaceState {
    pub fn load(store: &ConfigStore, profile_id: Uuid) -> Self {
        let result = store.load_containers();
        let error = result.as_ref().err().map(ToString::to_string);
        let draft = Container::new("", profile_id);
        Self {
            document: result.unwrap_or_default(),
            saved: draft.clone(),
            draft,
            selected: None,
            error,
        }
    }
    pub fn dirty(&self) -> bool {
        self.draft != self.saved
    }
    pub fn discard(&mut self) {
        self.draft = self.saved.clone();
    }
    pub fn select(&mut self, id: Option<Uuid>, profile_id: Uuid) {
        self.draft = id
            .and_then(|id| {
                self.document
                    .containers
                    .iter()
                    .find(|value| value.id == id)
                    .cloned()
            })
            .unwrap_or_else(|| Container::new("", profile_id));
        self.selected = id;
        self.saved = self.draft.clone();
    }
    pub fn save(&mut self, store: &ConfigStore) -> Result<(), String> {
        // Reload the public store so edits from the CLI and other workspaces are retained.
        let mut document = store.load_containers().map_err(|err| err.to_string())?;
        let saved = if let Some(id) = self.selected {
            let value = document
                .containers
                .iter_mut()
                .find(|value| value.id == id)
                .ok_or("工作区已不存在，请刷新")?;
            // Only these fields are editable here; preserve freshly loaded opaque metadata.
            value.name = self.draft.name.clone();
            value.profile_id = self.draft.profile_id;
            value.clone()
        } else {
            document.containers.push(self.draft.clone());
            self.draft.clone()
        };
        store
            .save_containers(&document)
            .map_err(|err| err.to_string())?;
        self.document = document;
        self.selected = Some(saved.id);
        self.draft = saved.clone();
        self.saved = saved;
        self.error = None;
        Ok(())
    }
}

impl EnvBoxApp {
    pub(super) fn save_workspace(&mut self) -> Task<Message> {
        match self.workspaces.save(&self.store) {
            Ok(()) => self.set_status(
                StatusKind::Success,
                "环境容器已保存；后续运行使用新的 Profile 快照",
            ),
            Err(err) => {
                self.workspaces.error = Some(err.clone());
                self.set_status(StatusKind::Error, format!("工作区保存失败：{err}"));
            }
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::storage_policy::{StorageAction, StorageRule, StorageTarget};
    use envbox_core::DnsProfile;

    #[test]
    fn gui_state_persists_and_retains_cli_edits_and_failed_draft() {
        let root = std::env::temp_dir().join(format!("aura-gui-workspace-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let profile_id = Uuid::from_u128(1);
        store
            .save_profiles(&envbox_storage::ProfileDocument {
                profiles: vec![EnvironmentProfile {
                    id: profile_id,
                    name: "US".into(),
                    locale: LocaleProfile {
                        locale_name: "en-US".into(),
                        ui_language: "en-US".into(),
                        region: "US".into(),
                    },
                    timezone: TimezoneProfile {
                        windows_id: "Pacific Standard Time".into(),
                        iana_id: "America/Los_Angeles".into(),
                    },
                    dns: DnsProfile {
                        mode: envbox_core::DnsMode::Host,
                        servers: vec![],
                        ..Default::default()
                    },
                    environment: Default::default(),
                    registry: Default::default(),
                    browser: Default::default(),
                    identity: Default::default(),
                }],
            })
            .unwrap();
        let mut state = WorkspaceState::load(&store, profile_id);
        state.draft.name = "GUI".into();
        assert!(state.dirty());
        state.save(&store).unwrap();
        assert!(!state.dirty());
        let id = state.draft.id;
        state.select(Some(id), profile_id);
        let mut external = store.load_containers().unwrap();
        let cli_workspace = Container::new("CLI", profile_id);
        external.containers.push(cli_workspace.clone());
        // Existing storage metadata remains opaque to the environment-only editor.
        external.containers[0]
            .storage_policy
            .rules
            .push(StorageRule {
                target: StorageTarget::RegistrySubtree,
                path: "HKCU\\Software\\FixtureVendor\\App".into(),
                action: StorageAction::SharedReadWrite,
            });
        external.containers[0].created_at_unix_ms = 123;
        let retained_policy = external.containers[0].storage_policy.clone();
        store.save_containers(&external).unwrap();
        assert_ne!(state.draft.storage_policy, retained_policy);
        state.draft.name = "GUI edit".into();
        state.save(&store).unwrap();
        assert_eq!(state.draft.storage_policy, retained_policy);
        assert_eq!(state.draft.created_at_unix_ms, 123);
        assert!(!state.dirty());
        assert!(store
            .load_containers()
            .unwrap()
            .containers
            .contains(&cli_workspace));
        let mut restarted = WorkspaceState::load(&store, profile_id);
        restarted.select(Some(id), profile_id);
        assert_eq!(restarted.draft.name, "GUI edit");
        assert_eq!(restarted.draft.storage_policy, retained_policy);
        restarted.draft.name = "renamed environment".into();
        restarted.save(&store).unwrap();
        assert_eq!(
            store.load_containers().unwrap().containers[0].storage_policy,
            retained_policy
        );
        restarted.draft.profile_id = Uuid::new_v4();
        let before = std::fs::read(store.containers_path()).unwrap();
        assert!(restarted.save(&store).is_err());
        assert!(restarted.dirty());
        assert_eq!(std::fs::read(store.containers_path()).unwrap(), before);
        restarted.discard();
        assert!(!restarted.dirty());
        let profiles = store.load_profiles().unwrap();
        store
            .save_profiles(&envbox_storage::ProfileDocument { profiles: vec![] })
            .unwrap();
        let mut missing_profile = WorkspaceState::load(&store, Uuid::nil());
        assert!(missing_profile.error.is_none());
        assert_eq!(missing_profile.document.containers.len(), 2);
        missing_profile.select(Some(id), Uuid::nil());
        assert_eq!(missing_profile.draft.profile_id, profile_id);
        assert!(store.prepare_run_snapshot(id, Uuid::new_v4()).is_err());
        store.save_profiles(&profiles).unwrap();
        std::fs::write(store.containers_path(), "invalid[").unwrap();
        let mut broken = WorkspaceState::load(&store, profile_id);
        assert!(broken.error.is_some());
        broken.draft.name = "cannot overwrite".into();
        assert!(broken.save(&store).is_err());
        assert_eq!(
            std::fs::read_to_string(store.containers_path()).unwrap(),
            "invalid["
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
