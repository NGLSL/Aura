use crate::{
    ManagedTarget, MemberRuntimeIdentity, ProcessIdentity, RunCommand, RunResult, RunView,
};
use envbox_core::{ConsoleHost, ContainerMode, LaunchTarget, RunSnapshot};
use envbox_launcher::{
    start_session_in_named_job_gated, InstanceJob, SessionHandle, SessionStartRequest,
};
use envbox_storage::ConfigStore;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct OwnedRun {
    retry_recovery: bool,
    _snapshot: Option<RunSnapshot>,
    _recovered_broker: Option<envbox_launcher::HostBroker>,
    command: RunCommand,
    result: RunResult,
    // These handles survive every management client disconnect.
    session: Option<SessionHandle>,
    job: Option<InstanceJob>,
    members: Vec<ManagedTarget>,
}

pub(crate) struct Runs {
    recovery_error: Option<String>,
    generation: String,
    store: ConfigStore,
    targets: Arc<Mutex<HashSet<ManagedTarget>>>,
    running: HashMap<Uuid, OwnedRun>,
    requests: HashMap<String, (RunCommand, String, RunResult)>,
    control_requests: HashMap<String, (String, Option<Uuid>, Option<RunCommand>, Vec<RunView>)>,
}
impl Runs {
    pub(crate) fn new(
        store: ConfigStore,
        targets: Arc<Mutex<HashSet<ManagedTarget>>>,
        generation: String,
    ) -> Self {
        let mut runs = Self {
            recovery_error: None,
            generation,
            store,
            targets,
            running: HashMap::new(),
            requests: HashMap::new(),
            control_requests: HashMap::new(),
        };
        runs.discover();
        runs
    }
    pub(crate) fn run(&mut self, request_id: &str, command: &RunCommand) -> (String, RunResult) {
        self.retry_scope(command.container_id);
        self.refresh();
        if let Some(error) = &self.recovery_error {
            return failed(command, error);
        }
        if self.control_requests.contains_key(request_id) {
            return failed(command, "request ID already belongs to a control operation");
        }
        if let Some((previous, status, result)) = self.requests.get(request_id) {
            if previous == command {
                if self.running.contains_key(&command.instance_id) {
                    return self.query(command);
                }
                return (status.clone(), result.clone());
            }
            return failed(
                command,
                "request ID is already bound to another transaction",
            );
        }
        if let Some(existing) = self.running.get(&command.instance_id) {
            if existing.command == *command {
                return self.query(command);
            }
            return failed(
                command,
                "Instance UUID already belongs to another application or Container",
            );
        }
        if self.running.values().any(|run| {
            run.command.container_id == command.container_id && run.result.state == "TrackingLost"
        }) {
            return failed(command, "workspace has TrackingLost ownership; reconcile original instances before a new Run");
        }
        let (status, result) = match self.start(command, request_id) {
            Ok(result) => ("Running".into(), result),
            Err(error) => failed(command, &error.to_string()),
        };
        self.requests.insert(
            request_id.into(),
            (command.clone(), status.clone(), result.clone()),
        );
        (status, result)
    }
    fn discover(&mut self) {
        let records = match crate::records::scan(&self.store) {
            Ok(records) => records,
            Err(error) => {
                self.recovery_error = Some(format!("cannot inspect persisted ownership: {error}"));
                return;
            }
        };
        let recovery_deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        for record in records {
            let mut result = record.result;
            if !matches!(result.state.as_str(), "Exited" | "Stopped") {
                result.state = "TrackingLost".into();
                if result.error.is_none() {
                    result.error = Some("old Supervisor ownership requires fresh Runtime reconfirmation and Job control".into());
                }
            }
            let mut run = OwnedRun {
                retry_recovery: false,
                _snapshot: record.snapshot,
                _recovered_broker: None,
                command: record.command,
                result,
                session: None,
                job: None,
                members: vec![],
            };
            if run.result.state == "TrackingLost" && run._snapshot.is_some() {
                if let Err(error) = restore(&mut run, &self.generation, recovery_deadline) {
                    run.retry_recovery = error.kind() == io::ErrorKind::TimedOut;
                    run.result.error =
                        Some(format!("recovery could not establish control: {error}"));
                } else {
                    let path = self
                        .store
                        .root()
                        .join("containers")
                        .join(run.command.container_id.to_string())
                        .join("runs")
                        .join(format!("{}.json", run.command.instance_id));
                    if let Err(error) = atomic_record(&path, &run.result) {
                        run.result.state = "TrackingLost".into();
                        run.result.error = Some(format!(
                            "reconfirmed ownership journal could not be sealed: {error}"
                        ));
                        run.job.take();
                        run._recovered_broker.take();
                    } else if let Ok(mut targets) = self.targets.lock() {
                        targets.extend(run.members.iter().copied());
                    }
                }
            }
            self.running.insert(run.command.instance_id, run);
        }
    }
    pub(crate) fn query(&mut self, command: &RunCommand) -> (String, RunResult) {
        self.retry_scope(command.container_id);
        self.refresh();
        match self.running.get(&command.instance_id) {
            Some(run) if run.command == *command => (run.result.state.clone(), run.result.clone()),
            _ => failed(
                command,
                "no matching owned Run; restart recovery is not implemented",
            ),
        }
    }
    pub(crate) fn refresh(&mut self) {
        for run in self.running.values_mut() {
            let Some(job) = run.job.as_ref() else {
                continue;
            };
            match job.stats() {
                Err(_) => {
                    run.result.state = "TrackingLost".into();
                }
                Ok(stats) if stats.active_processes == 0 => {
                    if run.result.state != "Stopping" {
                        run.result.state = "Exited".into();
                    } else {
                        run.result.state = "Stopped".into();
                    }
                    let path = self
                        .store
                        .root()
                        .join("containers")
                        .join(run.command.container_id.to_string())
                        .join("runs")
                        .join(format!("{}.json", run.command.instance_id));
                    if let Err(error) = atomic_record(&path, &run.result) {
                        run.result.error = Some(format!("final record persistence: {error}"));
                    }
                    if let Ok(mut targets) = self.targets.lock() {
                        for member in &run.members {
                            targets.remove(member);
                        }
                    }
                    run.members.clear();
                    run.session.take();
                    run._recovered_broker.take();
                    run.job.take();
                }
                Ok(stats) => {
                    let members: Vec<_> = stats
                        .process_ids
                        .into_iter()
                        .filter_map(|pid| ManagedTarget::observe(pid).ok())
                        .collect();
                    // Re-read membership so a PID observed after an exit/reuse is
                    // never registered as an owned target solely from a stale list.
                    if let Ok(confirmed) = job.stats() {
                        let members: Vec<_> = members
                            .into_iter()
                            .filter(|member| confirmed.process_ids.contains(&member.pid))
                            .collect();
                        if let Ok(mut targets) = self.targets.lock() {
                            for old in &run.members {
                                targets.remove(old);
                            }
                            targets.extend(members.iter().copied());
                        }
                        run.members = members;
                        let known: Vec<_> = run
                            .members
                            .iter()
                            .map(|member| ProcessIdentity {
                                pid: member.pid,
                                creation_time: member.creation_time,
                            })
                            .collect();
                        let facts = match member_facts(run, &run.members) {
                            Ok(facts) => facts,
                            Err(error) => {
                                if run.result.state != "Stopping" {
                                    run.result.state = "TrackingLost".into();
                                }
                                run.result.error =
                                    Some(format!("member Runtime verification: {error}"));
                                continue;
                            }
                        };
                        let verified_job = match job.stats() {
                            Ok(stats) => stats,
                            Err(error) => {
                                run.result.state = "TrackingLost".into();
                                run.result.error = Some(error.to_string());
                                continue;
                            }
                        };
                        if verified_job.process_ids.len() != run.members.len()
                            || run.members.iter().any(|member| {
                                !verified_job.process_ids.contains(&member.pid)
                                    || ManagedTarget::observe(member.pid).ok() != Some(*member)
                            })
                        {
                            run.result.state = "TrackingLost".into();
                            run.result.error =
                                Some("Job membership changed while sealing Runtime facts".into());
                            continue;
                        }
                        let next_state = if run.result.state == "Stopping" {
                            "Stopping"
                        } else {
                            "Running"
                        };
                        if known != run.result.known_members
                            || facts != run.result.member_runtimes
                            || run.result.state != next_state
                        {
                            let mut next = run.result.clone();
                            next.record_schema = 3;
                            next.known_members = known;
                            next.member_runtimes = facts;
                            next.state = next_state.into();
                            next.error = None;
                            let path = self
                                .store
                                .root()
                                .join("containers")
                                .join(run.command.container_id.to_string())
                                .join("runs")
                                .join(format!("{}.json", run.command.instance_id));
                            match atomic_record(&path, &next) {
                                Ok(()) => run.result = next,
                                Err(error) => {
                                    run.result.state = "TrackingLost".into();
                                    run.result.error =
                                        Some(format!("membership journal persistence: {error}"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    pub(crate) fn control(
        &mut self,
        request_id: &str,
        operation: &str,
        container: Option<Uuid>,
        command: Option<&RunCommand>,
    ) -> (String, Vec<RunView>) {
        if let Some(container) = container {
            self.retry_scope(container);
        }
        if self.recovery_error.is_some() {
            return ("StorageUnavailable".into(), vec![]);
        }
        if (operation == "Stop"
            && command.is_none_or(|target| container != Some(target.container_id)))
            || (operation == "StopAll" && (container.is_none() || command.is_some()))
            || (operation == "List" && command.is_some())
        {
            return ("InvalidRequest".into(), vec![]);
        }
        self.refresh();
        if let Some(id) = container {
            if !self
                .running
                .values()
                .any(|run| run.command.container_id == id)
            {
                match self.store.load_containers() {
                    Ok(document)
                        if document
                            .containers
                            .iter()
                            .any(|workspace| workspace.id == id) => {}
                    Ok(_) => return ("NotOwned".into(), vec![]),
                    Err(_) => return ("StorageUnavailable".into(), vec![]),
                }
            }
        }
        if operation != "List" {
            if self.requests.contains_key(request_id) {
                return ("RequestConflict".into(), vec![]);
            }
            if let Some((previous, scope, target, result)) = self.control_requests.get(request_id) {
                if previous == operation && *scope == container && target.as_ref() == command {
                    let current: Vec<_> = result
                        .iter()
                        .map(|view| {
                            self.running
                                .get(&view.result.instance_id)
                                .map(|run| RunView {
                                    result: run.result.clone(),
                                    process_ids: run
                                        .members
                                        .iter()
                                        .map(|member| member.pid)
                                        .collect(),
                                })
                                .unwrap_or_else(|| view.clone())
                        })
                        .collect();
                    return (control_status(&current).into(), current);
                }
                return ("RequestConflict".into(), vec![]);
            }
        }
        let mut selected: Vec<_> = self
            .running
            .iter()
            .filter(|(_, run)| {
                if let Some(target) = command {
                    run.command == *target
                } else {
                    container.is_none_or(|id| run.command.container_id == id)
                }
            })
            .map(|(id, _)| *id)
            .collect();
        selected.sort();
        if operation == "Stop" && (command.is_none() || selected.is_empty()) {
            return ("NotOwned".into(), vec![]);
        }
        if operation == "Stop"
            && selected.iter().any(|id| {
                self.running[id].job.is_none() && self.running[id].result.state == "TrackingLost"
            })
        {
            return ("NotControlled".into(), vec![]);
        }
        if operation == "StopAll" && container.is_none() {
            return ("InvalidRequest".into(), vec![]);
        }
        if matches!(operation, "Stop" | "StopAll") {
            for id in &selected {
                let run = self.running.get_mut(id).unwrap();
                if let Some(job) = run.job.as_ref() {
                    match job.terminate() {
                        Ok(()) => {
                            run.result.state = "Stopping".into();
                        }
                        Err(error) => {
                            run.result.state = "TrackingLost".into();
                            run.result.error = Some(error.to_string());
                        }
                    }
                }
            }
            let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                self.refresh();
                if selected.iter().all(|id| {
                    self.running[id].job.is_none() || self.running[id].result.state != "Stopping"
                }) || std::time::Instant::now() >= end
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let results: Vec<_> = selected
            .iter()
            .map(|id| {
                let run = &self.running[id];
                RunView {
                    result: run.result.clone(),
                    process_ids: run.members.iter().map(|member| member.pid).collect(),
                }
            })
            .collect();
        if operation != "List" {
            self.control_requests.insert(
                request_id.into(),
                (
                    operation.into(),
                    container,
                    command.cloned(),
                    results.clone(),
                ),
            );
        }
        (
            if operation == "List" {
                "Ok"
            } else {
                control_status(&results)
            }
            .into(),
            results,
        )
    }
    fn start(&mut self, command: &RunCommand, request_id: &str) -> io::Result<RunResult> {
        let invalid = |error: String| io::Error::new(io::ErrorKind::InvalidInput, error);
        let snapshot = self
            .store
            .load_run_snapshot(command.container_id, command.instance_id)
            .map_err(|error| invalid(error.to_string()))?;
        if !self
            .store
            .load_containers()
            .map_err(|error| invalid(error.to_string()))?
            .containers
            .iter()
            .any(|container| container.id == command.container_id)
        {
            return Err(invalid("Container UUID no longer exists".into()));
        }
        if !self
            .store
            .load_profiles()
            .map_err(|error| invalid(error.to_string()))?
            .profiles
            .iter()
            .any(|profile| profile.id == snapshot.effective_profile.id)
        {
            return Err(invalid(
                "snapshot source Profile was deleted; new Run refused".into(),
            ));
        }
        let applications = self
            .store
            .load_applications()
            .map_err(|error| invalid(error.to_string()))?;
        let application = applications
            .applications
            .into_iter()
            .find(|app| app.id == command.application_id)
            .ok_or_else(|| invalid("Application UUID not found".into()))?;
        application
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        validate_launch(&snapshot, &application)?;
        snapshot
            .effective_profile
            .dns
            .validate_runtime_support()
            .map_err(|error| invalid(error.to_string()))?;
        let expected = envbox_launcher::ipc::profile_to_message_with_flags(
            &snapshot.effective_profile,
            &command.instance_id.to_string(),
            true,
            application.audit,
        );
        let name = format!("Local\\AuraRun-{}-{}", command.instance_id, Uuid::new_v4());
        let job = InstanceJob::create_named_exclusive(&name)
            .map_err(|error| io::Error::other(error.to_string()))?;
        // An on-disk ownership record prevents a restarted server from silently
        // rebinding a possibly still-running Instance. Recovery is ticket 12.
        let record = self
            .store
            .root()
            .join("containers")
            .join(command.container_id.to_string())
            .join("runs")
            .join(format!("{}.json", command.instance_id));
        std::fs::create_dir_all(record.parent().unwrap())?;
        let pending = RunResult {
            record_schema: 3,
            request_id: request_id.into(),
            supervisor_generation: self.generation.clone(),
            job_name: name.clone(),
            snapshot_digest: snapshot.content_digest.clone(),
            container_id: command.container_id,
            instance_id: command.instance_id,
            application_id: command.application_id,
            profile_id: snapshot.effective_profile.id,
            runtime_module_path: Default::default(),
            runtime_module_sha256: String::new(),
            runtime_config_sha256: String::new(),
            runtime_version: String::new(),
            audit: application.audit,
            inherit_children: true,
            known_members: vec![],
            member_runtimes: vec![],
            root_pid: 0,
            creation_time: 0,
            mode: "compatibility".into(),
            entry_guarantee: "pending".into(),
            storage_policy_enforced: false,
            configuration_id: snapshot.configuration_id.clone(),
            error: None,
            state: "Starting".into(),
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&record)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&pending)?)?;
        file.sync_all()?;
        drop(file);
        let request = SessionStartRequest {
            application_id: application.id,
            launch: application.launch,
            arguments: application.arguments,
            working_directory: application.working_directory,
            profile: Some(snapshot.effective_profile.clone()),
            inherit_children: true,
            audit: application.audit,
        };
        let started = start_session_in_named_job_gated(request, command.instance_id, &name);
        let mut session = match started {
            Ok(session) => session,
            Err(error) => {
                return Err(self.cleanup_failed_start(
                    command,
                    &snapshot,
                    pending,
                    job,
                    &record,
                    error.to_string(),
                ));
            }
        };
        let verified = (|| -> io::Result<RunResult> {
            let observed = session.runtime_identity().ok_or_else(|| {
                io::Error::other("Runtime identity unavailable after gate release")
            })?;
            use sha2::{Digest, Sha256};
            let actual =
                envbox_launcher::IpcMessage::decode_line(&observed.identity.actual_profile)
                    .map_err(|error| io::Error::other(error.to_string()))?;
            let expected_hash = format!("{:x}", Sha256::digest(expected.encode_line().as_bytes()));
            if actual != expected
                || observed.config_sha256 != expected_hash
                || !observed.identity.config_complete
                || session.instance.id != command.instance_id
            {
                return Err(io::Error::other(
                    "actual Runtime Profile does not match immutable snapshot",
                ));
            }
            let identity = ManagedTarget::observe(session.instance.root_pid)?;
            if identity.creation_time != observed.identity.creation_time {
                return Err(io::Error::other("Runtime process generation mismatch"));
            }
            let result = RunResult {
                root_pid: identity.pid,
                creation_time: identity.creation_time,
                runtime_module_path: observed.identity.module_path.clone().into(),
                runtime_module_sha256: observed.module_sha256.clone(),
                runtime_config_sha256: observed.config_sha256.clone(),
                runtime_version: observed.identity.runtime_version.clone(),
                member_runtimes: vec![MemberRuntimeIdentity {
                    pid: identity.pid,
                    creation_time: identity.creation_time,
                    module_path: observed.identity.module_path.clone().into(),
                    module_sha256: observed.module_sha256.clone(),
                    config_sha256: observed.config_sha256.clone(),
                    runtime_version: observed.identity.runtime_version.clone(),
                }],
                known_members: vec![ProcessIdentity {
                    pid: identity.pid,
                    creation_time: identity.creation_time,
                }],
                entry_guarantee: "verified_pe_entry_no_tls".into(),
                state: "Running".into(),
                ..pending.clone()
            };
            // No Running response is published before this record is durable.
            atomic_record(&record, &result)?;
            self.targets
                .lock()
                .map_err(|_| io::Error::other("managed registry unavailable"))?
                .insert(identity);
            Ok(result)
        })();
        let result = match verified {
            Ok(result) => result,
            Err(error) => {
                return Err(self.cleanup_failed_start(
                    command,
                    &snapshot,
                    pending,
                    job,
                    &record,
                    error.to_string(),
                ));
            }
        };
        session.instance.container_id = Some(snapshot.container_id);
        session.instance.snapshot_id = Some(snapshot.snapshot_id);
        self.running.insert(
            command.instance_id,
            OwnedRun {
                retry_recovery: false,
                _snapshot: Some(snapshot),
                _recovered_broker: None,
                command: command.clone(),
                result: result.clone(),
                session: Some(session),
                job: Some(job),
                members: vec![ManagedTarget {
                    pid: result.root_pid,
                    creation_time: result.creation_time,
                }],
            },
        );
        Ok(result)
    }

    fn cleanup_failed_start(
        &mut self,
        command: &RunCommand,
        snapshot: &RunSnapshot,
        mut pending: RunResult,
        job: InstanceJob,
        record: &std::path::Path,
        cause: String,
    ) -> io::Error {
        let terminated = job.terminate();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let empty = loop {
            match job.stats() {
                Ok(stats) if stats.active_processes == 0 => break true,
                Ok(_) if std::time::Instant::now() < deadline && terminated.is_ok() => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                _ => break false,
            }
        };
        if empty {
            let _ = std::fs::remove_file(record);
            return io::Error::other(cause);
        }
        pending.state = "TrackingLost".into();
        pending.entry_guarantee = "unverified".into();
        pending.error = Some(format!(
            "{cause}; startup cleanup could not confirm empty Job"
        ));
        let _ = atomic_record(record, &pending);
        let error = pending.error.clone().unwrap();
        // Keep the exclusive Job owner available for a subsequent explicit Stop.
        // Neither a failed cleanup nor a disconnected client releases live ownership.
        self.running.insert(
            command.instance_id,
            OwnedRun {
                retry_recovery: false,
                _snapshot: Some(snapshot.clone()),
                _recovered_broker: None,
                command: command.clone(),
                result: pending,
                session: None,
                job: Some(job),
                members: vec![],
            },
        );
        io::Error::other(error)
    }
    fn retry_scope(&mut self, container_id: Uuid) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        for run in self
            .running
            .values_mut()
            .filter(|run| run.command.container_id == container_id && run.retry_recovery)
        {
            match restore(run, &self.generation, deadline) {
                Err(error) => {
                    run.retry_recovery = error.kind() == io::ErrorKind::TimedOut;
                    run.result.error = Some(format!(
                        "workspace recovery could not establish control: {error}"
                    ));
                }
                Ok(()) => {
                    run.retry_recovery = false;
                    let path = self
                        .store
                        .root()
                        .join("containers")
                        .join(container_id.to_string())
                        .join("runs")
                        .join(format!("{}.json", run.command.instance_id));
                    if let Err(error) = atomic_record(&path, &run.result) {
                        run.result.state = "TrackingLost".into();
                        run.result.error = Some(format!(
                            "reconfirmed ownership journal could not be sealed: {error}"
                        ));
                        run.job.take();
                        run._recovered_broker.take();
                    } else if let Ok(mut targets) = self.targets.lock() {
                        targets.extend(run.members.iter().copied());
                    }
                }
            }
        }
    }
}

fn control_status(results: &[RunView]) -> &'static str {
    if results.iter().any(|view| {
        view.result.error.is_some()
            || matches!(view.result.state.as_str(), "Stopping" | "TrackingLost")
    }) {
        "Partial"
    } else {
        "Ok"
    }
}

fn member_facts(
    run: &OwnedRun,
    members: &[ManagedTarget],
) -> io::Result<Vec<MemberRuntimeIdentity>> {
    use sha2::{Digest, Sha256};
    if members.is_empty()
        || members.len() > 256
        || run.result.entry_guarantee != "verified_pe_entry_no_tls"
    {
        return Err(io::Error::other(
            "Job member evidence is incomplete or exceeds bounded capacity",
        ));
    }
    let profile = &run
        ._snapshot
        .as_ref()
        .ok_or_else(|| io::Error::other("missing immutable Profile"))?
        .effective_profile;
    let expected = envbox_launcher::ipc::profile_to_message_with_flags(
        profile,
        &run.command.instance_id.to_string(),
        run.result.inherit_children,
        run.result.audit,
    );
    let config_sha256 = format!("{:x}", Sha256::digest(expected.encode_line().as_bytes()));
    let broker = run
        .session
        .as_ref()
        .and_then(|session| session.broker.as_ref())
        .or(run._recovered_broker.as_ref())
        .ok_or_else(|| io::Error::other("instance identity broker is unavailable"))?;
    let table = broker.table();
    let registry = table
        .lock()
        .map_err(|_| io::Error::other("instance registry poisoned"))?;
    let mut facts = Vec::with_capacity(members.len());
    for member in members {
        let observed = registry
            .validate_runtime(member.pid)
            .map_err(|error| io::Error::other(error.to_string()))?;
        if observed.identity.creation_time != member.creation_time
            || observed.identity.runtime_version != env!("CARGO_PKG_VERSION")
            || observed.config_sha256 != config_sha256
            || envbox_launcher::IpcMessage::decode_line(&observed.identity.actual_profile)
                .map_err(|error| io::Error::other(error.to_string()))?
                != expected
            || ManagedTarget::observe(member.pid).ok() != Some(*member)
        {
            return Err(io::Error::other(
                "actual member generation, version or full Profile does not match its instance",
            ));
        }
        facts.push(MemberRuntimeIdentity {
            pid: member.pid,
            creation_time: member.creation_time,
            module_path: observed.identity.module_path.clone().into(),
            module_sha256: observed.module_sha256.clone(),
            config_sha256: observed.config_sha256.clone(),
            runtime_version: observed.identity.runtime_version.clone(),
        });
    }
    facts.sort_by_key(|fact| (fact.pid, fact.creation_time));
    Ok(facts)
}

fn sealed_member_facts(
    result: &RunResult,
    members: &[ManagedTarget],
) -> io::Result<Vec<MemberRuntimeIdentity>> {
    if members.is_empty()
        || members.len() > 256
        || result.known_members.len() > 256
        || result.member_runtimes.len() > 256
    {
        return Err(io::Error::other(
            "sealed Job member evidence exceeds bounded capacity",
        ));
    }
    let mut known_pids = HashSet::new();
    if result.known_members.iter().any(|member| {
        member.pid == 0 || member.creation_time == 0 || !known_pids.insert(member.pid)
    }) {
        return Err(io::Error::other(
            "invalid or duplicate sealed process identity",
        ));
    }
    if result.record_schema == 2 {
        // The old root bundle is only a candidate. request_runtime_reconnect
        // still proves this exact file is already loaded in each actual Job
        // member. Opposite-architecture members cannot pass that proof.
        return Ok(members
            .iter()
            .map(|member| MemberRuntimeIdentity {
                pid: member.pid,
                creation_time: member.creation_time,
                module_path: result.runtime_module_path.clone(),
                module_sha256: result.runtime_module_sha256.clone(),
                config_sha256: result.runtime_config_sha256.clone(),
                runtime_version: result.runtime_version.clone(),
            })
            .collect());
    }
    let mut proof_pids = HashSet::new();
    if result.member_runtimes.iter().any(|fact| {
        fact.pid == 0
            || fact.creation_time == 0
            || !proof_pids.insert(fact.pid)
            || !result.known_members.contains(&ProcessIdentity {
                pid: fact.pid,
                creation_time: fact.creation_time,
            })
    }) || result.member_runtimes.len() != result.known_members.len()
    {
        return Err(io::Error::other(
            "sealed member Runtime proofs are missing, duplicate or generation-conflicting",
        ));
    }
    members
        .iter()
        .map(|member| {
            result
                .member_runtimes
                .iter()
                .find(|fact| fact.pid == member.pid && fact.creation_time == member.creation_time)
                .cloned()
                .ok_or_else(|| io::Error::other("actual Job member has no sealed Runtime identity"))
        })
        .collect()
}

/// Recovery grants control only after reopening the original kernel Job and
/// challenging every still-live sealed member through the original pipe.
fn restore(run: &mut OwnedRun, generation: &str, deadline: std::time::Instant) -> io::Result<()> {
    use envbox_launcher::{request_runtime_reconnect, session_pipe_name, HostBroker, SessionTable};
    use sha2::{Digest, Sha256};
    let result = &run.result;
    let snapshot = run
        ._snapshot
        .as_ref()
        .ok_or_else(|| io::Error::other("missing sealed snapshot"))?;
    if !matches!(result.record_schema, 2 | 3)
        || result.root_pid == 0
        || !result.inherit_children
        || result.mode != "compatibility"
        || result.entry_guarantee != "verified_pe_entry_no_tls"
        || snapshot.mode != ContainerMode::Compatibility
    {
        return Err(io::Error::other(
            "record lacks verified recoverable startup facts",
        ));
    }
    let prefix = format!("Local\\AuraRun-{}-", run.command.instance_id);
    if result
        .job_name
        .strip_prefix(&prefix)
        .and_then(|nonce| nonce.parse::<Uuid>().ok())
        .is_none()
    {
        return Err(io::Error::other("invalid scoped Job identity"));
    }
    let expected = envbox_launcher::ipc::profile_to_message_with_flags(
        &snapshot.effective_profile,
        &run.command.instance_id.to_string(),
        result.inherit_children,
        result.audit,
    );
    if format!("{:x}", Sha256::digest(expected.encode_line().as_bytes()))
        != result.runtime_config_sha256
    {
        return Err(io::Error::other(
            "sealed Runtime bundle or configuration hash changed",
        ));
    }
    let job = InstanceJob::open_named(&result.job_name)
        .map_err(|error| io::Error::other(error.to_string()))?;
    job.verify_tracking_limits()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let stats = job
        .stats()
        .map_err(|error| io::Error::other(error.to_string()))?;
    if stats.active_processes == 0 {
        // A sealed completed startup with an actual empty Job proves tree exit.
        run.result.state = "Exited".into();
        run.result.supervisor_generation = generation.into();
        run.result.error = None;
        return Ok(());
    }
    let members: Vec<_> = stats
        .process_ids
        .iter()
        .map(|pid| ManagedTarget::observe(*pid))
        .collect::<io::Result<_>>()?;
    if members.iter().any(|member| {
        !result.known_members.contains(&ProcessIdentity {
            pid: member.pid,
            creation_time: member.creation_time,
        })
    }) {
        return Err(io::Error::other(
            "Job members are not fully sealed or their process generations changed",
        ));
    }
    let sealed = sealed_member_facts(result, &members)?;
    let mut checked_bundles: HashMap<std::path::PathBuf, String> = HashMap::new();
    for fact in &sealed {
        if fact.config_sha256 != result.runtime_config_sha256
            || fact.runtime_version != env!("CARGO_PKG_VERSION")
        {
            return Err(io::Error::other(
                "sealed member configuration or Runtime version is unsupported",
            ));
        }
        let hash = if let Some(hash) = checked_bundles.get(&fact.module_path) {
            hash.clone()
        } else {
            let hash = bounded_bundle_hash(&fact.module_path, deadline)?;
            checked_bundles.insert(fact.module_path.clone(), hash.clone());
            hash
        };
        if hash != fact.module_sha256 {
            return Err(io::Error::other(
                "sealed member Runtime bundle hash changed",
            ));
        }
    }
    let mut table = SessionTable::new();
    table.set_instance_id(&run.command.instance_id.to_string());
    table.register_profile_flags(
        &snapshot.effective_profile,
        result.inherit_children,
        result.audit,
    );
    for fact in &sealed {
        table.bind_pid(fact.pid, &result.profile_id.to_string());
        table
            .expect_runtime(fact.pid, &fact.module_path)
            .map_err(|error| io::Error::other(error.to_string()))?;
    }
    let table = Arc::new(Mutex::new(table));
    let broker = HostBroker::start_on(
        table.clone(),
        session_pipe_name(&run.command.instance_id.to_string()),
    )?;
    for fact in &sealed {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|duration| duration.as_millis() >= 1)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "bounded recovery budget exhausted")
            })?;
        request_runtime_reconnect(
            fact.pid,
            fact.creation_time,
            &fact.module_path,
            &fact.module_sha256,
            remaining.min(std::time::Duration::from_secs(3)),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        let registry = table
            .lock()
            .map_err(|_| io::Error::other("recovery registry poisoned"))?;
        let observed = registry
            .validate_runtime(fact.pid)
            .map_err(|error| io::Error::other(error.to_string()))?;
        if !registry.runtime_reconfirmed(fact.pid)
            || observed.identity.creation_time != fact.creation_time
            || observed.module_sha256 != fact.module_sha256
            || observed.config_sha256 != fact.config_sha256
            || observed.identity.runtime_version != fact.runtime_version
            || envbox_launcher::IpcMessage::decode_line(&observed.identity.actual_profile)
                .map_err(|error| io::Error::other(error.to_string()))?
                != expected
        {
            return Err(io::Error::other(
                "fresh Runtime identity does not match sealed instance",
            ));
        }
    }
    let confirmed = job
        .stats()
        .map_err(|error| io::Error::other(error.to_string()))?;
    if confirmed.process_ids.len() != members.len()
        || members.iter().any(|member| {
            !confirmed.process_ids.contains(&member.pid)
                || ManagedTarget::observe(member.pid).ok() != Some(*member)
        })
    {
        return Err(io::Error::other(
            "Job membership changed during Runtime reconfirmation",
        ));
    }
    run.result.state = "Running".into();
    run.result.supervisor_generation = generation.into();
    run.result.error = None;
    run.result.record_schema = 3;
    run.result.member_runtimes = sealed;
    run.result.known_members = members
        .iter()
        .map(|member| ProcessIdentity {
            pid: member.pid,
            creation_time: member.creation_time,
        })
        .collect();
    run.members = members;
    run.job = Some(job);
    run._recovered_broker = Some(broker);
    Ok(())
}

fn bounded_bundle_hash(path: &std::path::Path, deadline: std::time::Instant) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::os::windows::fs::MetadataExt;
    use std::{
        io::Read,
        path::{Component, Prefix},
    };
    if std::time::Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "bounded recovery budget exhausted",
        ));
    }
    if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err(io::Error::other(
            "Runtime bundle must be a local disk file within recovery budget",
        ));
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.len() > 64 * 1024 * 1024
    {
        return Err(io::Error::other(
            "Runtime bundle is not a bounded plain PE file",
        ));
    }
    let mut file = std::fs::File::open(path)?.take(64 * 1024 * 1024 + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0usize;
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Runtime bundle hash exceeded recovery budget",
            ));
        }
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        total += size;
        if total > 64 * 1024 * 1024 {
            return Err(io::Error::other(
                "Runtime bundle exceeds bounded image size",
            ));
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn atomic_record(path: &std::path::Path, result: &RunResult) -> io::Result<()> {
    use std::io::Write;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let temporary = path.with_file_name(format!(".run-{}.tmp", Uuid::new_v4()));
    let write = (|| -> io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(result)?)?;
        file.sync_all()?;
        drop(file);
        let source: Vec<u16> = temporary
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let target: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            MoveFileExW(
                windows::core::PCWSTR(source.as_ptr()),
                windows::core::PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }?;
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    write
}

fn validate_launch(
    snapshot: &RunSnapshot,
    application: &envbox_core::Application,
) -> io::Result<()> {
    if snapshot.mode != ContainerMode::Compatibility {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Container and Strong isolation backends are unavailable",
        ));
    }
    if !application.inherit_children
        || application.console_host != ConsoleHost::Direct
        || matches!(application.launch, LaunchTarget::Packaged { .. })
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "workspace Run requires ordinary direct Win32 with child inheritance",
        ));
    }
    Ok(())
}
pub(crate) fn failed(command: &RunCommand, error: &str) -> (String, RunResult) {
    (
        "Failed".into(),
        RunResult {
            record_schema: 3,
            request_id: String::new(),
            supervisor_generation: String::new(),
            job_name: String::new(),
            snapshot_digest: String::new(),
            container_id: command.container_id,
            instance_id: command.instance_id,
            application_id: command.application_id,
            profile_id: Uuid::nil(),
            runtime_module_path: Default::default(),
            runtime_module_sha256: String::new(),
            runtime_config_sha256: String::new(),
            runtime_version: String::new(),
            audit: false,
            inherit_children: false,
            known_members: vec![],
            member_runtimes: vec![],
            root_pid: 0,
            creation_time: 0,
            mode: "unverified".into(),
            entry_guarantee: "unverified".into(),
            storage_policy_enforced: false,
            configuration_id: String::new(),
            error: Some(error.into()),
            state: "Failed".into(),
        },
    )
}

#[cfg(test)]
mod member_evidence_tests {
    use super::*;
    fn record(schema: u32) -> RunResult {
        let command = RunCommand {
            container_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
        };
        let mut result = failed(&command, "fixture").1;
        result.record_schema = schema;
        result.runtime_module_path = "D:/fixture/envbox-runtime64.dll".into();
        result.runtime_module_sha256 = "root-sha".into();
        result.runtime_config_sha256 = "config-sha".into();
        result.runtime_version = env!("CARGO_PKG_VERSION").into();
        result.known_members = vec![
            ProcessIdentity {
                pid: 1,
                creation_time: 11,
            },
            ProcessIdentity {
                pid: 2,
                creation_time: 22,
            },
        ];
        result.member_runtimes = result
            .known_members
            .iter()
            .map(|member| MemberRuntimeIdentity {
                pid: member.pid,
                creation_time: member.creation_time,
                module_path: if member.pid == 1 {
                    "D:/fixture/envbox-runtime64.dll"
                } else {
                    "D:/fixture/envbox-runtime32.dll"
                }
                .into(),
                module_sha256: format!("sha-{}", member.pid),
                config_sha256: "config-sha".into(),
                runtime_version: env!("CARGO_PKG_VERSION").into(),
            })
            .collect();
        result
    }
    #[test]
    fn surviving_child_uses_its_sealed_module_after_root_exit() {
        let result = record(3);
        let child = ManagedTarget {
            pid: 2,
            creation_time: 22,
        };
        let sealed = sealed_member_facts(&result, &[child]).unwrap();
        assert_eq!(sealed.len(), 1);
        assert_eq!(
            sealed[0].module_path,
            std::path::PathBuf::from("D:/fixture/envbox-runtime32.dll")
        );
        assert_eq!(sealed[0].module_sha256, "sha-2");
    }
    #[test]
    fn missing_duplicate_changed_generation_and_unknown_member_are_refused() {
        let child = ManagedTarget {
            pid: 2,
            creation_time: 22,
        };
        let mut missing = record(3);
        missing.member_runtimes.pop();
        assert!(sealed_member_facts(&missing, &[child]).is_err());
        let mut duplicate = record(3);
        duplicate.member_runtimes[1] = duplicate.member_runtimes[0].clone();
        assert!(sealed_member_facts(&duplicate, &[child]).is_err());
        let mut changed = record(3);
        changed.member_runtimes[1].creation_time += 1;
        assert!(sealed_member_facts(&changed, &[child]).is_err());
        assert!(sealed_member_facts(
            &record(3),
            &[ManagedTarget {
                pid: 3,
                creation_time: 33
            }]
        )
        .is_err());
    }
    #[test]
    fn schema_two_retains_exact_root_candidate_without_guessing_a_sibling() {
        let result = record(2);
        let sealed = sealed_member_facts(
            &result,
            &[ManagedTarget {
                pid: 2,
                creation_time: 22,
            }],
        )
        .unwrap();
        assert_eq!(sealed[0].module_path, result.runtime_module_path);
        assert_eq!(sealed[0].module_sha256, result.runtime_module_sha256);
    }
}
