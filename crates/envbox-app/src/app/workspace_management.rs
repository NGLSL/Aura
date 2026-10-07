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
    pending_profile_id: Option<Uuid>,
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
    fn clear_pending(&mut self) {
        self.pending_run = None;
        self.pending_profile_id = None;
    }

    pub fn pending_belongs_to_profile(&self, profile_id: Uuid) -> bool {
        self.pending_run.is_some() && self.pending_profile_id == Some(profile_id)
    }

    pub fn profile_instances(&self, profile_id: Uuid) -> impl Iterator<Item = &RunView> {
        self.instances
            .iter()
            .filter(move |view| view.result.profile_id == profile_id)
    }

    pub fn selection_changed(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.application_id = None;
    }

    fn accept(&mut self, reply: &ManagementReply, selected: Option<Uuid>) -> bool {
        self.busy = false;
        // Settle the sole in-flight Run even if the user navigated away.
        // Its records and errors still cannot overwrite the newly selected view.
        if reply.operation == "Run" && reply.result.is_err() && !reply.run_submission_unknown {
            self.clear_pending();
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
            self.clear_pending();
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
                    self.clear_pending();
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
                        self.clear_pending();
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
                        self.clear_pending();
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
}

fn prepare_run(
    store: &ConfigStore,
    container_id: Uuid,
    application_id: Uuid,
    instance_id: Uuid,
    expected_profile: Option<Uuid>,
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
    if expected_profile.is_some_and(|profile| profile != snapshot.effective_profile.id) {
        return Err("环境配置运行快照归属已变更；请刷新".into());
    }
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
        if self.workspaces.dirty() || self.profile_draft != self.profile_saved_draft {
            self.workspace_management.error =
                Some("请先保存或取消环境配置修改，再创建不可变运行快照".into());
            return Task::none();
        }
        if self.workspace_management.busy {
            return Task::none();
        }
        if self.workspace_management.pending_run.is_some() {
            self.workspace_management.error =
                Some("上次运行结果尚未知；请查询原实例，不能再次启动".into());
            return Task::none();
        }
        let Some(profile_id) = self.profile_draft.id else {
            self.workspace_management.error = Some("请先保存环境配置".into());
            return Task::none();
        };
        self.bind_profile_scope();
        if let Some(error) = self.workspaces.error.clone() {
            self.workspace_management.error = Some(format!("环境配置运行准备失败：{error}"));
            return Task::none();
        }
        let (Some(container_id), Some(application_id)) = (
            self.workspaces.selected,
            self.workspace_management.application_id,
        ) else {
            self.workspace_management.error = Some("请选择已保存环境配置及应用".into());
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
            self.workspace_management.error = Some("请先刷新运行状态，再启动环境配置".into());
            return Task::none();
        };
        self.workspace_management.pending_run = Some(command.clone());
        self.workspace_management.pending_profile_id = Some(profile_id);
        self.workspace_management.notice = None;
        self.workspace_request("Run", Some(command), Some(generation))
    }

    pub(super) fn workspace_run_status(&mut self) -> Task<Message> {
        let Some(command) = self.workspace_management.pending_run.clone() else {
            return Task::none();
        };
        if !self
            .profile_draft
            .id
            .is_some_and(|id| self.workspace_management.pending_belongs_to_profile(id))
        {
            return Task::none();
        }
        self.workspace_request(
            "RunStatus",
            Some(command),
            self.workspace_management.generation.clone(),
        )
    }

    pub(super) fn workspace_stop(&mut self, instance: Option<Uuid>) -> Task<Message> {
        let Some(profile_id) = self.profile_draft.id else {
            return Task::none();
        };
        let Some(generation) = self.workspace_management.generation.clone() else {
            self.workspace_management.error = Some("请先刷新当前 Supervisor 实例".into());
            return Task::none();
        };
        let command = if let Some(instance) = instance {
            let Some(view) = self
                .workspace_management
                .profile_instances(profile_id)
                .find(|view| {
                    view.result.instance_id == instance
                        && view.result.supervisor_generation == generation
                        && view.result.state != "TrackingLost"
                })
            else {
                self.workspace_management.error =
                    Some("实例不属于当前环境配置及运行管理会话；请刷新".into());
                return Task::none();
            };
            Some(RunCommand {
                container_id: view.result.container_id,
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
        let profile_id = self.profile_draft.id;
        let epoch = self.workspace_management.epoch;
        let operation = operation.to_string();
        let store = self.store.clone();
        Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                let worker_operation = operation.clone();
                std::thread::spawn(move || {
                    let result =
                        management_exchange(&store, &worker_operation, profile_id, run, generation);
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
                self.workspace_management.clear_pending();
            }
        }
        Task::none()
    }
}

fn profile_stop_commands(
    instances: &[RunView],
    profile: Uuid,
    generation: &str,
) -> Vec<RunCommand> {
    instances
        .iter()
        .filter(|view| {
            view.result.profile_id == profile
                && view.result.supervisor_generation == generation
                && view.result.state != "TrackingLost"
        })
        .map(|view| RunCommand {
            container_id: view.result.container_id,
            instance_id: view.result.instance_id,
            application_id: view.result.application_id,
        })
        .collect()
}

fn stop_profile_runs(
    profile: Uuid,
    generation: &str,
    mut exchange: impl FnMut(Request) -> Result<Response, String>,
) -> Result<Response, String> {
    let request = |command: &str, run: Option<RunCommand>| Request {
        version: PROTOCOL_VERSION,
        generation: Some(generation.into()),
        request_id: Uuid::new_v4().to_string(),
        command: command.into(),
        container_id: run.as_ref().map(|run| run.container_id),
        run,
    };
    let mut latest = exchange(request("List", None))?;
    if latest.generation != generation || latest.status != "Ok" {
        return Ok(latest);
    }
    let commands = profile_stop_commands(&latest.instances, profile, generation);
    let mut errors = Vec::new();
    for command in commands {
        match exchange(request("Stop", Some(command))) {
            Ok(response) => {
                if response.generation != generation {
                    // Keep facts already confirmed by the original service. A replacement
                    // invalidates the remaining stop set and has not verified those facts.
                    errors.push("运行管理会话已失效；停止操作已中断，请刷新当前会话".into());
                    latest.status = errors.join("；");
                    return Ok(latest);
                }
                for view in response.instances {
                    latest
                        .instances
                        .retain(|old| old.result.instance_id != view.result.instance_id);
                    latest.instances.push(view);
                }
                if !matches!(response.status.as_str(), "Ok" | "Stopped" | "Stopping") {
                    errors.push(format!("停止失败：{}", response.status));
                }
            }
            Err(error) => {
                errors.push(error);
                // Do not resend a stop after an uncertain transport failure.
                break;
            }
        }
    }
    match exchange(request("List", None)) {
        Ok(response) if response.generation == generation && response.status == "Ok" => {
            latest = response;
        }
        Ok(response) if response.generation != generation => {
            errors.push("运行管理会话已失效；请刷新当前会话，已确认的停止结果仍保留".into());
        }
        Ok(response) => errors.push(format!("刷新失败：{}", response.status)),
        Err(error) => errors.push(format!("刷新失败：{error}")),
    }
    if !errors.is_empty() {
        latest.status = errors.join("；");
    }
    Ok(latest)
}

#[cfg(windows)]
fn management_exchange(
    store: &ConfigStore,
    operation: &str,
    profile_id: Option<Uuid>,
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
            let profile_id = profile_id.ok_or("缺少环境配置身份")?;
            let document = store.load_containers().map_err(|error| error.to_string())?;
            if !document
                .containers
                .iter()
                .any(|scope| scope.id == target.container_id && scope.profile_id == profile_id)
            {
                return Err("环境配置运行作用域已变更；请刷新".into());
            }
            Some(prepare_run(
                store,
                target.container_id,
                target.application_id,
                target.instance_id,
                Some(profile_id),
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
        if operation == "StopAll" {
            return stop_profile_runs(
                profile_id.ok_or("缺少环境配置身份")?,
                &generation,
                |request| client.request(request).map_err(|error| error.to_string()),
            );
        }
        submitted = operation == "Run";
        client
            .request(Request {
                version: PROTOCOL_VERSION,
                generation: Some(generation),
                request_id: Uuid::new_v4().to_string(),
                command: operation.into(),
                container_id: command.as_ref().map(|run| run.container_id),
                run: command,
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
    use envbox_core::{LaunchTarget, LocaleProfile, TimezoneProfile};
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
    fn profile_membership_and_stop_set_use_snapshot_facts_across_legacy_scopes() {
        let profile = Uuid::from_u128(4);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut other = view(a, Uuid::new_v4(), "current");
        other.result.profile_id = Uuid::from_u128(5);
        let mut lost = view(a, Uuid::new_v4(), "current");
        lost.result.state = "TrackingLost".into();
        let current_a = view(a, Uuid::new_v4(), "current");
        let current_b = view(b, Uuid::new_v4(), "current");
        let old = view(a, Uuid::new_v4(), "old");
        let state = ManagementState {
            instances: vec![current_a.clone(), current_b.clone(), other, lost, old],
            ..Default::default()
        };
        assert_eq!(state.profile_instances(profile).count(), 4);
        let commands = profile_stop_commands(&state.instances, profile, "current");
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].container_id, a);
        assert_eq!(commands[1].container_id, b);
    }

    #[test]
    fn profile_stop_all_refreshes_and_retains_partial_success_when_transport_fails() {
        let profile = Uuid::from_u128(4);
        let a = view(Uuid::new_v4(), Uuid::new_v4(), "current");
        let b = view(Uuid::new_v4(), Uuid::new_v4(), "current");
        let mut other = view(a.result.container_id, Uuid::new_v4(), "current");
        other.result.profile_id = Uuid::from_u128(5);
        let initial = vec![a.clone(), b.clone(), other.clone()];
        let mut operations = vec![];
        let result = stop_profile_runs(profile, "current", |request| {
            operations.push(request.command.clone());
            assert_eq!(request.generation.as_deref(), Some("current"));
            match operations.len() {
                1 => {
                    assert!(request.container_id.is_none());
                    assert!(request.run.is_none());
                    Ok(reply(0, None, "current", initial.clone()).result.unwrap())
                }
                2 => {
                    assert_eq!(
                        request.run.as_ref().unwrap().instance_id,
                        a.result.instance_id
                    );
                    assert_eq!(request.container_id, Some(a.result.container_id));
                    let mut stopped = a.clone();
                    stopped.result.state = "Stopped".into();
                    let mut response = reply(0, None, "current", vec![stopped]).result.unwrap();
                    response.status = "Stopped".into();
                    Ok(response)
                }
                3 => {
                    assert_eq!(
                        request.run.as_ref().unwrap().instance_id,
                        b.result.instance_id
                    );
                    Err("stop timeout".into())
                }
                4 => {
                    assert!(request.container_id.is_none());
                    assert_eq!(request.command, "List");
                    Err("list timeout".into())
                }
                _ => panic!("unexpected extra request"),
            }
        })
        .unwrap();
        assert_eq!(operations, ["List", "Stop", "Stop", "List"]);
        assert_eq!(
            result
                .instances
                .iter()
                .find(|view| view.result.instance_id == a.result.instance_id)
                .unwrap()
                .result
                .state,
            "Stopped"
        );
        assert!(result
            .instances
            .iter()
            .any(|view| view.result.instance_id == other.result.instance_id
                && view.result.profile_id == other.result.profile_id));
        assert!(result.status.contains("stop timeout"));
        assert!(result.status.contains("list timeout"));
    }

    #[test]
    fn profile_stop_all_retains_confirmed_stop_when_final_list_is_rejected() {
        let profile = Uuid::from_u128(4);
        let running = view(Uuid::new_v4(), Uuid::new_v4(), "current");
        let mut count = 0;
        let response = stop_profile_runs(profile, "current", |request| {
            count += 1;
            match count {
                1 => Ok(reply(0, None, "current", vec![running.clone()])
                    .result
                    .unwrap()),
                2 => {
                    assert_eq!(request.command, "Stop");
                    let mut stopped = running.clone();
                    stopped.result.state = "Stopped".into();
                    let mut response = reply(0, None, "current", vec![stopped]).result.unwrap();
                    response.status = "Stopped".into();
                    Ok(response)
                }
                3 => {
                    assert_eq!(request.command, "List");
                    let mut rejected = reply(0, None, "current", vec![]).result.unwrap();
                    rejected.status = "StorageUnavailable".into();
                    Ok(rejected)
                }
                _ => panic!("unexpected request"),
            }
        })
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(response.instances.len(), 1);
        assert_eq!(response.instances[0].result.state, "Stopped");
        assert_eq!(response.generation, "current");
        assert!(response.status.contains("StorageUnavailable"));
    }

    #[test]
    fn profile_stop_all_preserves_old_confirmed_facts_and_aborts_after_service_replacement() {
        let profile = Uuid::from_u128(4);
        let a = view(Uuid::new_v4(), Uuid::new_v4(), "old");
        let b = view(Uuid::new_v4(), Uuid::new_v4(), "old");
        let c = view(Uuid::new_v4(), Uuid::new_v4(), "old");
        let mut count = 0;
        let response = stop_profile_runs(profile, "old", |request| {
            count += 1;
            match count {
                1 => Ok(reply(0, None, "old", vec![a.clone(), b.clone(), c.clone()])
                    .result
                    .unwrap()),
                2 => {
                    assert_eq!(request.command, "Stop");
                    let mut stopped = a.clone();
                    stopped.result.state = "Stopped".into();
                    let mut response = reply(0, None, "old", vec![stopped]).result.unwrap();
                    response.status = "Stopped".into();
                    Ok(response)
                }
                3 => {
                    assert_eq!(request.command, "Stop");
                    assert_eq!(request.run.unwrap().instance_id, b.result.instance_id);
                    let mut replaced = reply(0, None, "new", vec![]).result.unwrap();
                    replaced.status = "GenerationMismatch".into();
                    Ok(replaced)
                }
                _ => panic!("service replacement must abort all remaining stops"),
            }
        })
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(response.generation, "old");
        assert!(response.status.contains("会话已失效"));
        assert_eq!(
            response
                .instances
                .iter()
                .find(|view| view.result.instance_id == a.result.instance_id)
                .unwrap()
                .result
                .state,
            "Stopped"
        );
        assert_eq!(
            response
                .instances
                .iter()
                .find(|view| view.result.instance_id == c.result.instance_id)
                .unwrap()
                .result
                .state,
            "Running"
        );
    }

    #[test]
    fn deleting_profile_with_unknown_run_keeps_the_query_entry_and_original_uuid() {
        let root =
            std::env::temp_dir().join(format!("aura-profile-pending-delete-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (mut app, _) = EnvBoxApp::new();
        app.store = ConfigStore::new(&root);
        let profile = Uuid::from_u128(4);
        app.profile_draft.id = Some(profile);
        let command = RunCommand {
            container_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
        };
        app.workspace_management.pending_run = Some(command.clone());
        app.workspace_management.pending_profile_id = Some(profile);
        std::fs::write(app.store.profiles_path(), "invalid[").unwrap();
        let _ = app.delete_profile();
        assert!(app.status.contains("启动结果尚未知"));
        assert_eq!(app.profile_draft.id, Some(profile));
        assert_eq!(app.workspace_management.pending_run, Some(command));
        assert!(app.workspace_management.pending_belongs_to_profile(profile));
        assert_eq!(
            std::fs::read_to_string(app.store.profiles_path()).unwrap(),
            "invalid["
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_stop_all_never_stops_after_a_generation_mismatch() {
        let profile = Uuid::from_u128(4);
        let mut count = 0;
        let response = stop_profile_runs(profile, "old", |request| {
            count += 1;
            assert_eq!(request.command, "List");
            let mut response = reply(
                0,
                None,
                "new",
                vec![view(Uuid::new_v4(), Uuid::new_v4(), "new")],
            )
            .result
            .unwrap();
            response.status = "GenerationMismatch".into();
            Ok(response)
        })
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(response.generation, "new");
        assert_eq!(response.status, "GenerationMismatch");
    }

    #[test]
    fn pending_profile_follows_submission_not_current_scope_and_clears_on_settlement() {
        let profile = Uuid::from_u128(4);
        let command = RunCommand {
            container_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
        };
        let mut state = ManagementState {
            pending_run: Some(command.clone()),
            pending_profile_id: Some(profile),
            ..Default::default()
        };
        state.selection_changed();
        assert!(state.pending_belongs_to_profile(profile));
        assert!(!state.pending_belongs_to_profile(Uuid::from_u128(5)));
        let mut completed = reply(0, Some(command.container_id), "current", vec![]);
        completed.operation = "Run".into();
        completed.result.as_mut().unwrap().run =
            Some(view(command.container_id, command.instance_id, "current").result);
        assert!(!state.accept(&completed, Some(Uuid::new_v4())));
        assert!(state.pending_run.is_none());
        assert!(state.pending_profile_id.is_none());
        assert!(state.instances.is_empty());
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
        assert_eq!(state.profile_instances(Uuid::from_u128(4)).count(), 2);
        state.instances.push(view(b, Uuid::new_v4(), "old"));
        assert_eq!(state.profile_instances(Uuid::from_u128(4)).count(), 3);
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
            .profile_instances(Uuid::from_u128(4))
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
            identity: Default::default(),
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
            prepare_run(&store, container.id, application.id, instance, None).unwrap(),
            RunCommand {
                container_id: container.id,
                instance_id: instance,
                application_id: application.id
            }
        );
        assert!(prepare_run(
            &store,
            container.id,
            application.id,
            Uuid::new_v4(),
            Some(Uuid::new_v4())
        )
        .unwrap_err()
        .contains("快照归属已变更"));
        let path = store.run_snapshot_path(container.id, instance);
        let bytes = std::fs::read(&path).unwrap();
        let mut edited = profile;
        edited.name = "changed".into();
        store
            .save_profiles(&envbox_storage::ProfileDocument {
                profiles: vec![edited],
            })
            .unwrap();
        assert!(prepare_run(&store, container.id, application.id, instance, None).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(prepare_run(&store, container.id, Uuid::new_v4(), Uuid::new_v4(), None).is_err());
        store
            .save_profiles(&envbox_storage::ProfileDocument { profiles: vec![] })
            .unwrap();
        assert!(prepare_run(&store, container.id, application.id, Uuid::new_v4(), None).is_err());
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
    fn stop_cannot_address_another_profile_or_old_generation() {
        let (mut app, _) = EnvBoxApp::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let instance = Uuid::new_v4();
        app.workspaces.selected = Some(b);
        app.profile_draft.id = Some(Uuid::from_u128(4));
        app.workspace_management.busy = false;
        app.workspace_management.generation = Some("current".into());
        let mut other = view(a, instance, "current");
        other.result.profile_id = Uuid::from_u128(5);
        app.workspace_management.instances.push(other);
        let _ = app.workspace_stop(Some(instance));
        assert!(!app.workspace_management.busy);
        assert!(app
            .workspace_management
            .error
            .as_deref()
            .unwrap()
            .contains("不属于当前环境配置"));
        app.workspace_management.instances = vec![view(b, instance, "old")];
        let _ = app.workspace_stop(Some(instance));
        assert!(!app.workspace_management.busy);
    }
}
