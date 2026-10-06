//! Workspace operations use the authenticated Supervisor; the GUI owns no Job.
use super::*;
use envbox_supervisor::{Request, Response, RunCommand, RunView, PROTOCOL_VERSION};

#[derive(Default)]
pub struct ManagementState {
    pub application_id: Option<Uuid>,
    pub generation: Option<String>,
    pub instances: Vec<RunView>,
    pub busy: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub pending_run: Option<RunCommand>,
    epoch: u64,
}

#[derive(Debug, Clone)]
pub struct ManagementReply {
    pub epoch: u64,
    pub workspace_id: Option<Uuid>,
    pub operation: String,
    pub result: Result<Response, String>,
    pub run_submission_unknown: bool,
}

impl ManagementState {
    pub fn selection_changed(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.application_id = None;
    }

    fn accept(&mut self, reply: &ManagementReply, selected: Option<Uuid>) -> bool {
        self.busy = false;
        // Settle the sole in-flight Run even if the user navigated away.
        // Its records and errors still cannot overwrite the newly selected view.
        if reply.operation == "Run" && reply.result.is_err() && !reply.run_submission_unknown {
            self.pending_run = None;
            self.notice = None;
        }
        if reply
            .result
            .as_ref()
            .ok()
            .and_then(|response| response.run.as_ref())
            .is_some_and(|run| {
                self.pending_run
                    .as_ref()
                    .is_some_and(|pending| pending.instance_id == run.instance_id)
                    && run.state != "Starting"
            })
        {
            self.pending_run = None;
            self.notice = None;
        }
        // A delayed A response must never update B's workspace view.
        if reply.epoch != self.epoch || reply.workspace_id != selected {
            return false;
        }
        match &reply.result {
            Err(error) => self.error = Some(error.clone()),
            Ok(response) => {
                self.error = None;
                let changed = self
                    .generation
                    .as_ref()
                    .is_some_and(|old| old != &response.generation);
                if changed {
                    self.instances.clear();
                    self.pending_run = None;
                    self.notice = Some("Supervisor 已换代；列表以当前服务端记录为准。TrackingLost 表示未恢复跟踪，不能保证进程已退出。".into());
                }
                self.generation = Some(response.generation.clone());
                if reply.operation == "List" {
                    self.instances = response.instances.clone();
                    if self.pending_run.as_ref().is_some_and(|pending| {
                        self.instances.iter().any(|view| {
                            view.result.instance_id == pending.instance_id
                                && view.result.state != "Starting"
                        })
                    }) {
                        self.pending_run = None;
                        self.notice = None;
                    }
                } else if let Some(run) = &response.run {
                    self.instances
                        .retain(|view| view.result.instance_id != run.instance_id);
                    self.instances.push(RunView {
                        result: run.clone(),
                        process_ids: vec![],
                    });
                    if run.state != "Starting" {
                        self.pending_run = None;
                    }
                } else if matches!(reply.operation.as_str(), "Stop" | "StopAll") {
                    for view in &response.instances {
                        self.instances
                            .retain(|old| old.result.instance_id != view.result.instance_id);
                        self.instances.push(view.clone());
                    }
                }
                if let Some(error) = response.run.as_ref().and_then(|run| run.error.clone()) {
                    self.error = Some(error);
                } else if !matches!(
                    response.status.as_str(),
                    "Ok" | "Running" | "Stopped" | "Stopping"
                ) {
                    self.error = Some(format!("Supervisor：{}", response.status));
                }
            }
        }
        true
    }

    pub fn workspace_instances(&self, workspace: Uuid) -> impl Iterator<Item = &RunView> {
        // Current List responses may include recovered metadata whose original
        // generation is retained when the Job could not be reopened.
        self.instances
            .iter()
            .filter(move |view| view.result.container_id == workspace)
    }
}

fn prepare_run(
    store: &ConfigStore,
    container_id: Uuid,
    application_id: Uuid,
    instance_id: Uuid,
) -> Result<RunCommand, String> {
    let applications = store
        .load_applications()
        .map_err(|error| error.to_string())?;
    let application = applications
        .applications
        .iter()
        .find(|application| application.id == application_id)
        .ok_or("所选持久应用已不存在；请刷新")?;
    application.validate().map_err(|error| error.to_string())?;
    let snapshot = store
        .prepare_run_snapshot(container_id, instance_id)
        .map_err(|error| error.to_string())?;
    snapshot
        .effective_profile
        .dns
        .validate_runtime_support()
        .map_err(|error| error.to_string())?;
    Ok(RunCommand {
        container_id,
        instance_id,
        application_id,
    })
}

impl EnvBoxApp {
    pub(super) fn workspace_list(&mut self) -> Task<Message> {
        self.workspace_request("List", None, None)
    }

    pub(super) fn workspace_run(&mut self) -> Task<Message> {
        if self.workspaces.dirty() {
            self.workspace_management.error =
                Some("请先保存或取消工作区修改，再创建不可变运行快照".into());
            return Task::none();
        }
        if self.workspace_management.pending_run.is_some() {
            self.workspace_management.error =
                Some("上次运行结果尚未知；请查询原实例，不能再次启动".into());
            return Task::none();
        }
        let (Some(container_id), Some(application_id)) = (
            self.workspaces.selected,
            self.workspace_management.application_id,
        ) else {
            self.workspace_management.error = Some("请选择已保存工作区及持久应用".into());
            return Task::none();
        };
        let command = RunCommand {
            container_id,
            application_id,
            instance_id: Uuid::new_v4(),
        };
        if self.workspace_management.busy {
            return Task::none();
        }
        let Some(generation) = self.workspace_management.generation.clone() else {
            self.workspace_management.error = Some("请先刷新 Supervisor，再启动工作区".into());
            return Task::none();
        };
        self.workspace_management.pending_run = Some(command.clone());
        self.workspace_management.notice = None;
        self.workspace_request("Run", Some(command), Some(generation))
    }

    pub(super) fn workspace_run_status(&mut self) -> Task<Message> {
        let Some(command) = self.workspace_management.pending_run.clone() else {
            return Task::none();
        };
        if Some(command.container_id) != self.workspaces.selected {
            return Task::none();
        }
        self.workspace_request(
            "RunStatus",
            Some(command),
            self.workspace_management.generation.clone(),
        )
    }

    pub(super) fn workspace_stop(&mut self, instance: Option<Uuid>) -> Task<Message> {
        let Some(container_id) = self.workspaces.selected else {
            return Task::none();
        };
        let Some(generation) = self.workspace_management.generation.clone() else {
            self.workspace_management.error = Some("请先刷新当前 Supervisor 实例".into());
            return Task::none();
        };
        let command = if let Some(instance) = instance {
            let Some(view) = self
                .workspace_management
                .workspace_instances(container_id)
                .find(|view| {
                    view.result.instance_id == instance
                        && view.result.supervisor_generation == generation
                        && view.result.state != "TrackingLost"
                })
            else {
                self.workspace_management.error =
                    Some("实例不属于当前工作区及 Supervisor 代；请刷新".into());
                return Task::none();
            };
            Some(RunCommand {
                container_id,
                instance_id: instance,
                application_id: view.result.application_id,
            })
        } else {
            None
        };
        self.workspace_request(
            if instance.is_some() {
                "Stop"
            } else {
                "StopAll"
            },
            command,
            Some(generation),
        )
    }

    fn workspace_request(
        &mut self,
        operation: &str,
        run: Option<RunCommand>,
        generation: Option<String>,
    ) -> Task<Message> {
        if self.workspace_management.busy {
            return Task::none();
        }
        self.workspace_management.busy = true;
        self.workspace_management.error = None;
        let workspace_id = self.workspaces.selected;
        let epoch = self.workspace_management.epoch;
        let operation = operation.to_string();
        let store = self.store.clone();
        Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                let worker_operation = operation.clone();
                std::thread::spawn(move || {
                    let result = management_exchange(
                        &store,
                        &worker_operation,
                        workspace_id,
                        run,
                        generation,
                    );
                    let _ = tx.send(result);
                });
                let (result, run_submission_unknown) = rx
                    .await
                    .unwrap_or_else(|_| (Err("Supervisor 后台请求异常终止".into()), true));
                ManagementReply {
                    epoch,
                    workspace_id,
                    operation,
                    result,
                    run_submission_unknown,
                }
            },
            Message::WorkspaceManagementResult,
        )
    }

    pub(super) fn finish_workspace_management(&mut self, reply: ManagementReply) -> Task<Message> {
        let accepted = self
            .workspace_management
            .accept(&reply, self.workspaces.selected);
        if !accepted {
            return self.workspace_list();
        }
        if reply.operation == "Run" && reply.result.is_err() {
            if reply.run_submission_unknown {
                self.workspace_management.notice = Some(
                    "运行结果未知；保留原 Instance UUID，请查询原实例，禁止自动重新启动。".into(),
                );
            } else {
                self.workspace_management.pending_run = None;
            }
        }
        Task::none()
    }
}

#[cfg(windows)]
fn management_exchange(
    store: &ConfigStore,
    operation: &str,
    workspace_id: Option<Uuid>,
    run: Option<RunCommand>,
    generation: Option<String>,
) -> (Result<Response, String>, bool) {
    let mut submitted = false;
    let result = (|| {
        let mut client = envbox_supervisor::SupervisorClient::beside_current_executable()
            .map_err(|error| error.to_string())?;
        client.timeout = std::time::Duration::from_secs(20);
        let command = if operation == "Run" {
            let target = run.as_ref().ok_or("缺少运行身份")?;
            Some(prepare_run(
                store,
                target.container_id,
                target.application_id,
                target.instance_id,
            )?)
        } else {
            run
        };
        let generation = match generation {
            Some(generation) => generation,
            None => {
                client
                    .ensure_started()
                    .map_err(|error| error.to_string())?
                    .generation
            }
        };
        submitted = operation == "Run";
        client
            .request(Request {
                version: PROTOCOL_VERSION,
                generation: Some(generation),
                request_id: Uuid::new_v4().to_string(),
                command: operation.into(),
                run: command,
                container_id: workspace_id,
            })
            .map_err(|error| error.to_string())
    })();
    let unknown = submitted && result.is_err();
    (result, unknown)
}

#[cfg(not(windows))]
fn management_exchange(
    _: &ConfigStore,
    _: &str,
    _: Option<Uuid>,
    _: Option<RunCommand>,
    _: Option<String>,
) -> (Result<Response, String>, bool) {
    (Err("Supervisor 仅支持 Windows".into()), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_supervisor::RunResult;

    fn view(container: Uuid, instance: Uuid, generation: &str) -> RunView {
        RunView {
            process_ids: vec![42],
            result: RunResult {
                record_schema: 2,
                request_id: Uuid::new_v4().to_string(),
                supervisor_generation: generation.into(),
                job_name: "fixture".into(),
                snapshot_digest: "digest".into(),
                container_id: container,
                instance_id: instance,
                application_id: Uuid::from_u128(3),
                root_pid: 42,
                profile_id: Uuid::from_u128(4),
                runtime_module_path: Default::default(),
                runtime_module_sha256: String::new(),
                runtime_config_sha256: String::new(),
                runtime_version: String::new(),
                environment_facts: None,
                audit: false,
                inherit_children: true,
                known_members: vec![],
                member_runtimes: vec![],
                creation_time: 5,
                mode: "compatibility".into(),
                entry_guarantee: "verified_pe_entry_no_tls".into(),
                storage_policy_enforced: false,
                configuration_id: "config".into(),
                error: None,
                state: "Running".into(),
            },
        }
    }

    fn reply(
        epoch: u64,
        workspace: Option<Uuid>,
        generation: &str,
        instances: Vec<RunView>,
    ) -> ManagementReply {
        ManagementReply {
            epoch,
            workspace_id: workspace,
            operation: "List".into(),
            run_submission_unknown: false,
            result: Ok(Response {
                version: PROTOCOL_VERSION,
                generation: generation.into(),
                request_id: "fixture".into(),
                supervisor_pid: 10,
                status: "Ok".into(),
                run: None,
                instances,
            }),
        }
    }

    #[test]
    fn late_workspace_reply_cannot_replace_current_scope() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let mut state = ManagementState::default();
        state.busy = true;
        state.selection_changed();
        assert!(!state.accept(
            &reply(0, Some(a), "old", vec![view(a, Uuid::new_v4(), "old")]),
            Some(b)
        ));
        assert!(!state.busy);
        assert!(state.instances.is_empty());
        assert!(state.generation.is_none());
        assert!(state.accept(
            &reply(
                1,
                Some(b),
                "current",
                vec![
                    view(a, Uuid::new_v4(), "current"),
                    view(b, Uuid::new_v4(), "current")
                ]
            ),
            Some(b)
        ));
        assert_eq!(state.workspace_instances(b).count(), 1);
        state.instances.push(view(b, Uuid::new_v4(), "old"));
        assert_eq!(state.workspace_instances(b).count(), 2);
    }

    #[test]
    fn current_list_displays_restored_lost_record_and_its_original_generation() {
        let container = Uuid::new_v4();
        let mut lost = view(container, Uuid::new_v4(), "previous");
        lost.result.state = "TrackingLost".into();
        lost.result.error = Some("Job could not be reopened".into());
        let mut state = ManagementState::default();
        state.accept(
            &reply(0, Some(container), "current", vec![lost]),
            Some(container),
        );
        let visible = state
            .workspace_instances(container)
            .next()
            .expect("restored Lost record must remain visible");
        assert_eq!(visible.result.supervisor_generation, "previous");
        assert_eq!(visible.result.state, "TrackingLost");
        assert_eq!(
            visible.result.error.as_deref(),
            Some("Job could not be reopened")
        );
    }

    #[test]
    fn generation_refresh_discards_old_instances_and_unknown_run() {
        let container = Uuid::new_v4();
        let mut state = ManagementState::default();
        state.generation = Some("old".into());
        state.instances.push(view(container, Uuid::new_v4(), "old"));
        state.pending_run = Some(RunCommand {
            container_id: container,
            instance_id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
        });
        state.accept(&reply(0, Some(container), "new", vec![]), Some(container));
        assert!(state.instances.is_empty());
        assert!(state.pending_run.is_none());
        assert_eq!(state.generation.as_deref(), Some("new"));
        assert!(state.notice.as_deref().unwrap().contains("TrackingLost"));
        assert!(state.notice.as_deref().unwrap().contains("未恢复跟踪"));
    }

    #[test]
    fn late_run_result_settles_pending_identity_without_updating_new_workspace() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let instance = Uuid::new_v4();
        let mut state = ManagementState::default();
        state.pending_run = Some(RunCommand {
            container_id: a,
            instance_id: instance,
            application_id: Uuid::from_u128(3),
        });
        state.selection_changed();
        let mut completed = reply(0, Some(a), "generation", vec![]);
        completed.operation = "Run".into();
        completed.result.as_mut().unwrap().run = Some(view(a, instance, "generation").result);
        assert!(!state.accept(&completed, Some(b)));
        assert!(state.pending_run.is_none());
        assert!(state.instances.is_empty());
        assert!(state.generation.is_none());
    }

    #[test]
    fn public_prepare_uses_persisted_application_and_immutable_snapshot() {
        let root = std::env::temp_dir().join(format!("aura-gui-run-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let profile = EnvironmentProfile {
            id: Uuid::new_v4(),
            name: "fixture".into(),
            locale: LocaleProfile {
                locale_name: "en-US".into(),
                ui_language: "en-US".into(),
                region: "US".into(),
            },
            timezone: TimezoneProfile {
                windows_id: "Pacific Standard Time".into(),
                iana_id: "America/Los_Angeles".into(),
            },
            dns: Default::default(),
            environment: Default::default(),
            registry: Default::default(),
            browser: Default::default(),
        };
        store
            .save_profiles(&envbox_storage::ProfileDocument {
                profiles: vec![profile.clone()],
            })
            .unwrap();
        let container = envbox_core::Container::new("fixture", profile.id);
        store
            .save_containers(&envbox_storage::ContainerDocument {
                schema_version: 1,
                containers: vec![container.clone()],
            })
            .unwrap();
        let application = Application {
            id: Uuid::new_v4(),
            name: "fixture".into(),
            launch: LaunchTarget::Executable {
                path: "fixture.exe".into(),
            },
            working_directory: None,
            arguments: vec![],
            default_profile_id: profile.id,
            inherit_children: true,
            console_host: ConsoleHost::Direct,
            audit: false,
        };
        store
            .save_applications(&envbox_storage::ApplicationDocument {
                applications: vec![application.clone()],
            })
            .unwrap();
        let instance = Uuid::new_v4();
        assert_eq!(
            prepare_run(&store, container.id, application.id, instance).unwrap(),
            RunCommand {
                container_id: container.id,
                instance_id: instance,
                application_id: application.id
            }
        );
        let path = store.run_snapshot_path(container.id, instance);
        let bytes = std::fs::read(&path).unwrap();
        let mut edited = profile;
        edited.name = "changed".into();
        store
            .save_profiles(&envbox_storage::ProfileDocument {
                profiles: vec![edited],
            })
            .unwrap();
        assert!(prepare_run(&store, container.id, application.id, instance).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(prepare_run(&store, container.id, Uuid::new_v4(), Uuid::new_v4()).is_err());
        store
            .save_profiles(&envbox_storage::ProfileDocument { profiles: vec![] })
            .unwrap();
        assert!(prepare_run(&store, container.id, application.id, Uuid::new_v4()).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gui_dirty_and_busy_state_blocks_run_before_any_io() {
        let (mut app, _) = EnvBoxApp::new();
        app.workspaces.draft.name = "unsaved".into();
        let _ = app.workspace_run();
        assert!(app
            .workspace_management
            .error
            .as_deref()
            .unwrap()
            .contains("请先保存"));
        assert!(app.workspace_management.pending_run.is_none());
        app.workspaces.discard();
        app.workspaces.selected = Some(Uuid::new_v4());
        app.workspace_management.application_id = Some(Uuid::new_v4());
        app.workspace_management.generation = Some("fixture".into());
        app.workspace_management.busy = true;
        let _ = app.workspace_run();
        assert!(app.workspace_management.pending_run.is_none());
    }

    #[test]
    fn failed_run_keeps_identity_only_when_submission_result_is_unknown() {
        let (mut app, _) = EnvBoxApp::new();
        app.workspaces.discard();
        let container = Uuid::new_v4();
        let command = RunCommand {
            container_id: container,
            instance_id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
        };
        app.workspaces.selected = Some(container);
        app.workspace_management.pending_run = Some(command.clone());
        let error = ManagementReply {
            epoch: 0,
            workspace_id: Some(container),
            operation: "Run".into(),
            result: Err("timeout".into()),
            run_submission_unknown: true,
        };
        let _ = app.finish_workspace_management(error.clone());
        assert_eq!(app.workspace_management.pending_run, Some(command));
        assert!(app
            .workspace_management
            .notice
            .as_deref()
            .unwrap()
            .contains("禁止自动重新启动"));
        let _ = app.workspace_run();
        assert!(app
            .workspace_management
            .error
            .as_deref()
            .unwrap()
            .contains("结果尚未知"));
        let mut validation_error = error;
        validation_error.run_submission_unknown = false;
        let _ = app.finish_workspace_management(validation_error);
        assert!(app.workspace_management.pending_run.is_none());
        assert!(app.workspace_management.notice.is_none());
    }

    #[test]
    fn stop_cannot_address_another_workspace_or_old_generation() {
        let (mut app, _) = EnvBoxApp::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let instance = Uuid::new_v4();
        app.workspaces.selected = Some(b);
        app.workspace_management.busy = false;
        app.workspace_management.generation = Some("current".into());
        app.workspace_management
            .instances
            .push(view(a, instance, "current"));
        let _ = app.workspace_stop(Some(instance));
        assert!(!app.workspace_management.busy);
        assert!(app
            .workspace_management
            .error
            .as_deref()
            .unwrap()
            .contains("不属于当前工作区"));
        app.workspace_management.instances = vec![view(b, instance, "old")];
        let _ = app.workspace_stop(Some(instance));
        assert!(!app.workspace_management.busy);
    }
}
