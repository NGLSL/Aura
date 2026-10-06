use crate::{
    EnvironmentRuntimeFacts, InstalledHookFact, ManagedTarget, MemberRuntimeIdentity,
    ProcessIdentity, RunCommand, RunResult, RunView,
};
use envbox_core::{ConsoleHost, ContainerMode, LaunchTarget, RunSnapshot};
use envbox_launcher::{
    start_session_in_named_job_gated, InstanceJob, SessionHandle, SessionStartRequest,
};
use envbox_storage::ConfigStore;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Holds an immutable Runtime image open without FILE_SHARE_DELETE while a
/// managed run can still need it. The content-addressed staging cache is
/// outside the installation directory, but the lease also makes an in-use
/// image resistant to an installer or cleanup pass that tries to remove it.
struct BundleLease {
    path: PathBuf,
    identity: FileIdentity,
    sha256: String,
    _file: std::fs::File,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    volume_serial: u32,
    file_index: u64,
}

fn handle_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information)
            .map_err(|_| io::Error::last_os_error())?;
    }
    Ok(FileIdentity {
        volume_serial: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x0000_0400 != 0
}

fn check_bundle_budget(deadline: std::time::Instant) -> io::Result<()> {
    if std::time::Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Runtime bundle recovery budget exhausted",
        ));
    }
    Ok(())
}

fn check_local_bundle_path(path: &std::path::Path, deadline: std::time::Instant) -> io::Result<()> {
    use std::path::{Component, Prefix};
    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDriveTypeW};
    check_bundle_budget(deadline)?;
    let mut components = path.components();
    let drive = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Err(io::Error::other("Runtime bundle must be a local disk file")),
        },
        _ => return Err(io::Error::other("Runtime bundle must be a local disk file")),
    };
    if components.next() != Some(Component::RootDir)
        || components.any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::other(
            "Runtime bundle path must be absolute without parent traversal",
        ));
    }
    let root: Vec<u16> = format!("{}:\\", char::from(drive))
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let drive_type = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
    check_bundle_budget(deadline)?;
    if !matches!(drive_type, 2 | 3) {
        return Err(io::Error::other(
            "Runtime bundle cannot use a network or unknown disk",
        ));
    }
    // Reject junctions/symlinks in the directories too, before canonicalize
    // could follow one onto a network path.
    let ancestors: Vec<_> = path.ancestors().collect();
    for ancestor in ancestors.into_iter().rev() {
        check_bundle_budget(deadline)?;
        let metadata = std::fs::symlink_metadata(ancestor)?;
        check_bundle_budget(deadline)?;
        if is_reparse(&metadata) {
            return Err(io::Error::other(
                "Runtime bundle path contains a reparse point",
            ));
        }
    }
    Ok(())
}

fn open_stable_bundle(
    path: &std::path::Path,
    deadline: std::time::Instant,
) -> io::Result<(std::fs::File, FileIdentity)> {
    check_local_bundle_path(path, deadline)?;
    let before = std::fs::symlink_metadata(path)?;
    check_bundle_budget(deadline)?;
    if is_reparse(&before) || !before.is_file() {
        return Err(io::Error::other(
            "Runtime bundle path is a reparse point or not a regular file",
        ));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT: never follow a replaced leaf.
        .open(path)?;
    check_bundle_budget(deadline)?;
    let identity = handle_identity(&file)?;
    check_bundle_budget(deadline)?;
    let opened = file.metadata()?;
    check_bundle_budget(deadline)?;
    if is_reparse(&opened) || !opened.is_file() {
        return Err(io::Error::other(
            "opened Runtime bundle is not a bounded regular file",
        ));
    }

    // Re-open the path only as an identity probe. The returned `file` remains
    // the lease handle; if the path changed between the first open and this
    // probe, the two file IDs differ and the operation fails closed.
    let probe = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)?;
    check_bundle_budget(deadline)?;
    let probe_identity = handle_identity(&probe)?;
    check_bundle_budget(deadline)?;
    let after = std::fs::symlink_metadata(path)?;
    check_bundle_budget(deadline)?;
    if is_reparse(&after) || probe_identity != identity {
        return Err(io::Error::other(
            "Runtime bundle path identity changed during open",
        ));
    }
    Ok((file, identity))
}

fn hash_bundle_handle(
    file: &mut std::fs::File,
    deadline: std::time::Instant,
) -> io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;

    check_bundle_budget(deadline)?;
    let metadata = file.metadata()?;
    check_bundle_budget(deadline)?;
    if !metadata.is_file() || is_reparse(&metadata) || metadata.len() > MAX_BUNDLE_BYTES {
        return Err(io::Error::other(
            "Runtime bundle is not a bounded regular file",
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    check_bundle_budget(deadline)?;
    hash_bundle_stream(file.take(MAX_BUNDLE_BYTES + 1), deadline)
}

fn hash_bundle_stream(
    mut input: impl std::io::Read,
    deadline: std::time::Instant,
) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        check_bundle_budget(deadline)?;
        let count = input.read(&mut buffer)?;
        check_bundle_budget(deadline)?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > 64 * 1024 * 1024 {
            return Err(io::Error::other("Runtime bundle exceeds 64 MiB"));
        }
        hash.update(&buffer[..count]);
        check_bundle_budget(deadline)?;
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Cooperative budget checks surround synchronous local filesystem operations.
/// Windows open/metadata/read calls cannot be interrupted by this deadline; an
/// individual stalled OS call may return after it, at which point we fail closed.
fn retain_bundles(
    leases: &mut Vec<BundleLease>,
    facts: &[MemberRuntimeIdentity],
    deadline: std::time::Instant,
) -> io::Result<()> {
    retain_bundles_with_canonicalize(leases, facts, deadline, |path| std::fs::canonicalize(path))
}

fn retain_bundles_with_canonicalize(
    leases: &mut Vec<BundleLease>,
    facts: &[MemberRuntimeIdentity],
    deadline: std::time::Instant,
    mut canonicalize: impl FnMut(&std::path::Path) -> io::Result<PathBuf>,
) -> io::Result<()> {
    check_bundle_budget(deadline)?;
    for fact in facts {
        // Pin the authenticated path before resolving aliases: canonicalization
        // must never silently switch ownership to a different file with equal bytes.
        let (mut file, identity) = open_stable_bundle(&fact.module_path, deadline)?;
        let path = canonicalize(&fact.module_path)?;
        check_bundle_budget(deadline)?;
        let (_probe, canonical_identity) = open_stable_bundle(&path, deadline)?;
        if canonical_identity != identity {
            return Err(io::Error::other(
                "Runtime bundle identity changed during canonicalization",
            ));
        }
        if let Some(lease) = leases.iter().find(|lease| lease.path == path) {
            if identity != lease.identity {
                return Err(io::Error::other(
                    "Runtime bundle path identity changed while retained",
                ));
            }
            if lease.sha256 != fact.module_sha256 {
                return Err(io::Error::other(
                    "Runtime bundle hash conflicts with an existing lease",
                ));
            }
            continue;
        }
        // Deliberately share read only. Omitting FILE_SHARE_WRITE and
        // FILE_SHARE_DELETE keeps
        // both a live instance's exact bundle from becoming delete-pending and
        // a concurrent writer from replacing its bytes while its Runtime may
        // still be mapped in a target process.
        let sha256 = hash_bundle_handle(&mut file, deadline)?;
        if sha256 != fact.module_sha256 {
            return Err(io::Error::other(
                "Runtime bundle hash does not match authenticated Runtime identity",
            ));
        }
        leases.push(BundleLease {
            path,
            identity,
            sha256,
            _file: file,
        });
    }
    Ok(())
}

fn process_generation_is_absent<F>(
    identities: &[ProcessIdentity],
    mut observe: F,
) -> io::Result<bool>
where
    F: FnMut(u32) -> io::Result<ManagedTarget>,
{
    if identities.is_empty() {
        return Ok(false);
    }
    for identity in identities {
        let expected = ManagedTarget {
            pid: identity.pid,
            creation_time: identity.creation_time,
        };
        match observe(identity.pid) {
            Ok(current) if current != expected => {}
            Ok(_) => return Ok(false),
            Err(error) if process_is_absent_error(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

fn process_is_absent_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error.raw_os_error().is_some_and(|code| {
            // The windows crate commonly exposes HRESULT_FROM_WIN32(code)
            // through io::Error, so inspect the low Win32 word as well as the
            // direct positive error representation.
            matches!((code as u32) & 0xffff, 2 | 3 | 6 | 87 | 1168)
        })
}

/// Classify whether the last sealed member generations are gone. This is only
/// diagnostic evidence for a missing named Job; the caller still stays
/// TrackingLost because a PID list cannot prove that an unknown descendant did
/// not escape the lost Job.
fn sealed_members_are_absent(result: &RunResult) -> io::Result<bool> {
    if !sealed_member_evidence_complete(result) {
        return Ok(false);
    }
    process_generation_is_absent(&result.known_members, ManagedTarget::observe)
}

fn sealed_member_evidence_complete(result: &RunResult) -> bool {
    if result.record_schema != 3
        || result.known_members.is_empty()
        || result.member_runtimes.len() != result.known_members.len()
    {
        return false;
    }
    let mut known = HashSet::new();
    if result.known_members.iter().any(|member| {
        member.pid == 0
            || member.creation_time == 0
            || !known.insert((member.pid, member.creation_time))
    }) {
        return false;
    }
    let mut facts = HashSet::new();
    result.member_runtimes.iter().all(|fact| {
        fact.pid != 0
            && fact.creation_time != 0
            && !fact.module_path.as_os_str().is_empty()
            && !fact.module_sha256.is_empty()
            && !fact.config_sha256.is_empty()
            && !fact.runtime_version.is_empty()
            && known.contains(&(fact.pid, fact.creation_time))
            && facts.insert((fact.pid, fact.creation_time))
    })
}

struct OwnedRun {
    stop_failed: bool,
    retry_recovery: bool,
    _snapshot: Option<RunSnapshot>,
    _recovered_broker: Option<envbox_launcher::HostBroker>,
    _bundle_leases: Vec<BundleLease>,
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
                stop_failed: false,
                retry_recovery: false,
                _snapshot: record.snapshot,
                _recovered_broker: None,
                _bundle_leases: Vec::new(),
                command: record.command,
                result,
                session: None,
                job: None,
                members: vec![],
            };
            if run.result.state == "TrackingLost" && run._snapshot.is_some() {
                if let Err(error) = restore(&mut run, &self.generation, recovery_deadline) {
                    run.retry_recovery = error.kind() == io::ErrorKind::TimedOut;
                    run.result.state = "TrackingLost".into();
                    run.result.error =
                        Some(format!("recovery could not establish control: {error}"));
                    let path = self
                        .store
                        .root()
                        .join("containers")
                        .join(run.command.container_id.to_string())
                        .join("runs")
                        .join(format!("{}.json", run.command.instance_id));
                    if let Err(persist) = atomic_record(&path, &run.result) {
                        run.result.error = Some(format!(
                            "{}; recovery loss record could not be persisted: {persist}",
                            run.result.error.as_deref().unwrap_or("recovery failed")
                        ));
                    }
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
            let path = self
                .store
                .root()
                .join("containers")
                .join(run.command.container_id.to_string())
                .join("runs")
                .join(format!("{}.json", run.command.instance_id));
            match job.stats() {
                Err(error) => {
                    persist_tracking_loss(
                        &path,
                        &mut run.result,
                        format!("Job inspection failed: {error}"),
                        false,
                    );
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
                    run._bundle_leases.clear();
                    run.job.take();
                }
                Ok(stats) => {
                    // Only an explicit successful Stop retry or fresh recovery can
                    // clear a failed termination; membership alone proves no control.
                    if run.stop_failed {
                        continue;
                    }
                    let members: Vec<_> = stats
                        .process_ids
                        .into_iter()
                        .filter_map(|pid| ManagedTarget::observe(pid).ok())
                        .collect();
                    // Re-read membership so a PID observed after an exit/reuse is
                    // never registered as an owned target solely from a stale list.
                    let confirmed = match job.stats() {
                        Ok(confirmed) => confirmed,
                        Err(error) => {
                            persist_tracking_loss(
                                &path,
                                &mut run.result,
                                format!("Job membership confirmation failed: {error}"),
                                false,
                            );
                            continue;
                        }
                    };
                    {
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
                                persist_tracking_loss(
                                    &path,
                                    &mut run.result,
                                    format!("member Runtime verification: {error}"),
                                    true,
                                );
                                continue;
                            }
                        };
                        if let Err(error) = retain_bundles(
                            &mut run._bundle_leases,
                            &facts,
                            std::time::Instant::now() + std::time::Duration::from_secs(4),
                        ) {
                            persist_tracking_loss(
                                &path,
                                &mut run.result,
                                format!("Runtime bundle retention failed: {error}"),
                                true,
                            );
                            continue;
                        }
                        let verified_job = match job.stats() {
                            Ok(stats) => stats,
                            Err(error) => {
                                persist_tracking_loss(
                                    &path,
                                    &mut run.result,
                                    format!("Job Runtime sealing inspection failed: {error}"),
                                    false,
                                );
                                continue;
                            }
                        };
                        if verified_job.process_ids.len() != run.members.len()
                            || run.members.iter().any(|member| {
                                !verified_job.process_ids.contains(&member.pid)
                                    || ManagedTarget::observe(member.pid).ok() != Some(*member)
                            })
                        {
                            persist_tracking_loss(
                                &path,
                                &mut run.result,
                                "Job membership changed while sealing Runtime facts".into(),
                                false,
                            );
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
                            next.environment_facts = root_environment_facts(&next);
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
                                    persist_tracking_loss(
                                        &path,
                                        &mut run.result,
                                        format!("membership journal persistence: {error}"),
                                        false,
                                    );
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
        self.control_with_terminate(request_id, operation, container, command, |job| {
            job.terminate()
                .map_err(|error| io::Error::other(error.to_string()))
        })
    }
    fn control_with_terminate(
        &mut self,
        request_id: &str,
        operation: &str,
        container: Option<Uuid>,
        command: Option<&RunCommand>,
        mut terminate: impl FnMut(&InstanceJob) -> io::Result<()>,
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
                    match terminate(job) {
                        Ok(()) => {
                            if run.stop_failed {
                                run.result.error = None;
                            }
                            run.stop_failed = false;
                            run.result.state = "Stopping".into();
                        }
                        Err(error) => {
                            run.stop_failed = true;
                            let path = self
                                .store
                                .root()
                                .join("containers")
                                .join(run.command.container_id.to_string())
                                .join("runs")
                                .join(format!("{}.json", run.command.instance_id));
                            persist_tracking_loss(
                                &path,
                                &mut run.result,
                                format!("Job termination failed: {error}"),
                                false,
                            );
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
            environment_facts: None,
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
                environment_facts: Some(environment_facts(&observed.identity)),
                member_runtimes: vec![MemberRuntimeIdentity {
                    pid: identity.pid,
                    creation_time: identity.creation_time,
                    module_path: observed.identity.module_path.clone().into(),
                    module_sha256: observed.module_sha256.clone(),
                    config_sha256: observed.config_sha256.clone(),
                    runtime_version: observed.identity.runtime_version.clone(),
                    environment_facts: Some(environment_facts(&observed.identity)),
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
        let mut bundle_leases = Vec::new();
        if let Err(error) = retain_bundles(
            &mut bundle_leases,
            &result.member_runtimes,
            std::time::Instant::now() + std::time::Duration::from_secs(4),
        ) {
            return Err(self.cleanup_failed_start(
                command,
                &snapshot,
                pending,
                job,
                &record,
                format!("cannot retain active Runtime bundle: {error}"),
            ));
        }
        session.instance.container_id = Some(snapshot.container_id);
        session.instance.snapshot_id = Some(snapshot.snapshot_id);
        self.running.insert(
            command.instance_id,
            OwnedRun {
                stop_failed: false,
                retry_recovery: false,
                _snapshot: Some(snapshot),
                _recovered_broker: None,
                _bundle_leases: bundle_leases,
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
        pending: RunResult,
        job: InstanceJob,
        record: &std::path::Path,
        cause: String,
    ) -> io::Error {
        self.cleanup_failed_start_with_terminate(
            command,
            snapshot,
            pending,
            job,
            record,
            cause,
            |job| {
                job.terminate()
                    .map_err(|error| io::Error::other(error.to_string()))
            },
        )
    }
    fn cleanup_failed_start_with_terminate(
        &mut self,
        command: &RunCommand,
        snapshot: &RunSnapshot,
        mut pending: RunResult,
        job: InstanceJob,
        record: &std::path::Path,
        cause: String,
        terminate: impl FnOnce(&InstanceJob) -> io::Result<()>,
    ) -> io::Error {
        let terminated = terminate(&job);
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
        pending.entry_guarantee = "unverified".into();
        persist_tracking_loss(
            record,
            &mut pending,
            format!("{cause}; startup cleanup could not confirm empty Job"),
            false,
        );
        let error = pending.error.clone().unwrap();
        // Keep the exclusive Job owner available for a subsequent explicit Stop.
        // Neither a failed cleanup nor a disconnected client releases live ownership.
        self.running.insert(
            command.instance_id,
            OwnedRun {
                stop_failed: terminated.is_err(),
                retry_recovery: false,
                _snapshot: Some(snapshot.clone()),
                _recovered_broker: None,
                _bundle_leases: Vec::new(),
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
                    run.result.state = "TrackingLost".into();
                    run.result.error = Some(format!(
                        "workspace recovery could not establish control: {error}"
                    ));
                    let path = self
                        .store
                        .root()
                        .join("containers")
                        .join(container_id.to_string())
                        .join("runs")
                        .join(format!("{}.json", run.command.instance_id));
                    if let Err(persist) = atomic_record(&path, &run.result) {
                        run.result.error = Some(format!(
                            "{}; recovery loss record could not be persisted: {persist}",
                            run.result.error.as_deref().unwrap_or("recovery failed")
                        ));
                    }
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

/// Call only after broker authentication and immutable Profile equality checks.
fn environment_facts(identity: &envbox_launcher::ipc::RuntimeIdentity) -> EnvironmentRuntimeFacts {
    EnvironmentRuntimeFacts {
        config_complete: identity.config_complete,
        profile_matches_snapshot: true,
        hooks: identity
            .hooks
            .iter()
            .map(|(group, count)| InstalledHookFact {
                group: group.clone(),
                attached_api_count: *count,
            })
            .collect(),
    }
}

fn root_environment_facts(result: &RunResult) -> Option<EnvironmentRuntimeFacts> {
    result
        .member_runtimes
        .iter()
        .find(|member| {
            member.pid == result.root_pid && member.creation_time == result.creation_time
        })
        .and_then(|member| member.environment_facts.clone())
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
            environment_facts: Some(environment_facts(&observed.identity)),
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
                environment_facts: None,
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
    // Work from an immutable snapshot of the journal while this function
    // transitions `run` to Exited or Running below.
    let result = run.result.clone();
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
    let job = match InstanceJob::open_named(&result.job_name) {
        Ok(job) => job,
        Err(error)
            if matches!(
                error,
                envbox_launcher::job::JobError::Open(2 | 3 | 6 | 87 | 1168)
            ) =>
        {
            // A missing named Job is never enough to conclude that the whole
            // tree exited. Job Objects do not give us a durable descendant
            // list after the last handle is gone, and an unsealed descendant
            // may still be alive. Keep the run TrackingLost even when all
            // sealed generations are absent; the extra distinction makes the
            // UI/record explain why no safe Stop or new Run is granted.
            let detail = if sealed_members_are_absent(&result).unwrap_or(false) {
                "named tracking Job no longer exists; sealed generations are absent but the complete process tree cannot be proven"
            } else {
                "named tracking Job no longer exists; process tree membership cannot be proven"
            };
            return Err(io::Error::other(format!("{detail} ({error})")));
        }
        Err(error) => return Err(io::Error::other(error.to_string())),
    };
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
        run._bundle_leases.clear();
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
    let sealed = sealed_member_facts(&result, &members)?;
    let mut fresh_facts = Vec::with_capacity(sealed.len());
    for fact in &sealed {
        if fact.config_sha256 != result.runtime_config_sha256
            || fact.runtime_version != env!("CARGO_PKG_VERSION")
        {
            return Err(io::Error::other(
                "sealed member configuration or Runtime version is unsupported",
            ));
        }
    }
    // Acquire the leases before reconnecting any target. If a later
    // reconnection check fails, the run remains TrackingLost but its exact
    // bundle paths stay protected while the original process generations are
    // still present.
    retain_bundles(&mut run._bundle_leases, &sealed, deadline)?;
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
        let mut fresh = fact.clone();
        fresh.environment_facts = Some(environment_facts(&observed.identity));
        fresh_facts.push(fresh);
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
    run.result.member_runtimes = fresh_facts;
    run.result.environment_facts = root_environment_facts(&run.result);
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

/// Keep the last sealed member identities; failed verification must not seal
/// newly observed members as recoverable ownership evidence.
fn persist_tracking_loss(
    path: &std::path::Path,
    result: &mut RunResult,
    reason: String,
    preserve_stopping: bool,
) {
    if !preserve_stopping || result.state != "Stopping" {
        result.state = "TrackingLost".into();
    }
    result.error = Some(reason);
    if let Err(error) = atomic_record(path, result) {
        result.error = Some(format!(
            "{}; ownership loss journal persistence failed: {error}",
            result
                .error
                .as_deref()
                .unwrap_or("ownership tracking failed")
        ));
    }
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
            "Container and Strong storage/access isolation modes are outside the current Compatibility environment-information container scope",
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
            environment_facts: None,
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

    #[test]
    fn legacy_records_do_not_invent_environment_facts() {
        let mut value = serde_json::to_value(record(3)).unwrap();
        value.as_object_mut().unwrap().remove("environment_facts");
        for member in value["member_runtimes"].as_array_mut().unwrap() {
            member.as_object_mut().unwrap().remove("environment_facts");
        }
        let restored: RunResult = serde_json::from_value(value).unwrap();
        assert!(restored.environment_facts.is_none());
        assert!(restored
            .member_runtimes
            .iter()
            .all(|member| member.environment_facts.is_none()));
        let legacy_candidates = sealed_member_facts(
            &record(2),
            &[ManagedTarget {
                pid: 1,
                creation_time: 11,
            }],
        )
        .unwrap();
        assert!(legacy_candidates[0].environment_facts.is_none());
    }

    #[test]
    fn observed_hook_counts_survive_durable_record_without_implying_coverage() {
        let identity = envbox_launcher::ipc::RuntimeIdentity {
            pid: 1,
            creation_time: 11,
            protocol: envbox_launcher::ipc::RUNTIME_IDENTITY_PROTOCOL,
            runtime_version: env!("CARGO_PKG_VERSION").into(),
            module_path: "fixture.dll".into(),
            actual_profile: "fixture profile already verified by caller".into(),
            config_complete: true,
            hooks: vec![
                ("locale".into(), 14),
                ("dns".into(), 2),
                ("future_group".into(), 0),
            ],
        };
        let facts = environment_facts(&identity);
        assert!(facts.config_complete && facts.profile_matches_snapshot);
        assert_eq!(facts.hooks[2].attached_api_count, 0);
        let mut result = record(3);
        result.root_pid = 1;
        result.creation_time = 11;
        result.member_runtimes[0].environment_facts = Some(facts.clone());
        result.environment_facts = root_environment_facts(&result);
        let root = std::env::temp_dir().join(format!("aura-environment-facts-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("run.json");
        atomic_record(&path, &result).unwrap();
        let restored: RunResult = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.environment_facts, Some(facts.clone()));
        assert_eq!(restored.member_runtimes[0].environment_facts, Some(facts));
        result.creation_time = 12;
        assert!(root_environment_facts(&result).is_none());
        result.member_runtimes.remove(0);
        assert!(root_environment_facts(&result).is_none());
        let cleanup_root = std::fs::canonicalize(&root).unwrap();
        let temporary_root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(cleanup_root.is_absolute());
        assert_eq!(cleanup_root.parent(), Some(temporary_root.as_path()));
        assert!(cleanup_root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("aura-environment-facts-"));
        std::fs::remove_dir_all(cleanup_root).unwrap();
    }

    #[test]
    fn bundle_resolution_is_bound_to_the_original_stable_file_identity() {
        use sha2::Digest;
        let root = std::env::temp_dir().join(format!("aura-lease-resolution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("original.dll");
        let replacement = root.join("replacement.dll");
        std::fs::write(&original, b"same fixture bytes").unwrap();
        std::fs::write(&replacement, b"same fixture bytes").unwrap();
        let mut fact = record(3).member_runtimes.remove(0);
        fact.module_path = original.clone();
        fact.module_sha256 = format!("{:x}", sha2::Sha256::digest(b"same fixture bytes"));
        let mut leases = Vec::new();
        let error = retain_bundles_with_canonicalize(
            &mut leases,
            &[fact],
            std::time::Instant::now() + std::time::Duration::from_secs(4),
            |path| {
                assert_eq!(path, original);
                assert!(
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(&original)
                        .is_err(),
                    "original must be pinned before resolution"
                );
                assert!(
                    std::fs::remove_file(&original).is_err(),
                    "original must deny replacement before resolution"
                );
                // A resolver/directory race returns another actual file containing
                // the same bytes. A matching hash cannot authorize its different ID.
                std::fs::canonicalize(&replacement)
            },
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("identity changed during canonicalization"));
        assert!(leases.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_stop_is_persisted_and_stays_lost_until_an_explicit_retry() {
        use std::os::windows::process::CommandExt;
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{CloseHandle, HANDLE},
                System::JobObjects::{OpenJobObjectW, TerminateJobObject},
            },
        };
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        struct QueryHandle(HANDLE);
        impl Drop for QueryHandle {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let root = std::env::temp_dir().join(format!("aura-stop-failure-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let mut result = record(3);
        result.state = "Running".into();
        result.error = None;
        let command = RunCommand {
            container_id: result.container_id,
            instance_id: result.instance_id,
            application_id: result.application_id,
        };
        let path = root
            .join("containers")
            .join(command.container_id.to_string())
            .join("runs")
            .join(format!("{}.json", command.instance_id));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        atomic_record(&path, &result).unwrap();
        let child = Child(
            std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 60",
                ])
                .creation_flags(0x0800_0000)
                .spawn()
                .unwrap(),
        );
        let name = format!("Local\\AuraStopTest-{}", Uuid::new_v4());
        let mut job = InstanceJob::create_named_exclusive(&name).unwrap();
        job.assign_pid(child.0.id()).unwrap();
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let query_only =
            QueryHandle(unsafe { OpenJobObjectW(0x0004, false, PCWSTR(wide.as_ptr())).unwrap() });
        let mut runs = Runs {
            recovery_error: None,
            generation: "fixture".into(),
            store,
            targets: Arc::new(Mutex::new(HashSet::new())),
            running: HashMap::new(),
            requests: HashMap::new(),
            control_requests: HashMap::new(),
        };
        runs.running.insert(
            command.instance_id,
            OwnedRun {
                stop_failed: false,
                retry_recovery: false,
                _snapshot: None,
                _recovered_broker: None,
                _bundle_leases: vec![],
                command: command.clone(),
                result,
                session: None,
                job: Some(job),
                members: vec![],
            },
        );
        let response = runs.control_with_terminate(
            "denied-stop",
            "Stop",
            Some(command.container_id),
            Some(&command),
            |_| {
                unsafe { TerminateJobObject(query_only.0, 1) }
                    .map_err(|_| io::Error::last_os_error())
            },
        );
        assert_eq!(response.0, "Partial");
        let sealed: RunResult = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(sealed.state, "TrackingLost");
        assert!(sealed
            .error
            .as_deref()
            .unwrap()
            .contains("Job termination failed"));
        runs.refresh();
        assert_eq!(
            runs.running[&command.instance_id].result.error, sealed.error,
            "refresh must retain the termination failure"
        );
        assert_eq!(
            runs.control(
                "denied-stop",
                "Stop",
                Some(command.container_id),
                Some(&command)
            )
            .0,
            "Partial"
        );
        // The same real access-denied termination also exercises startup cleanup.
        // Locking the journal makes its atomic replacement fail on Windows.
        let owned = runs.running.remove(&command.instance_id).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let profile = envbox_core::EnvironmentProfile {
            id: Uuid::new_v4(),
            name: "cleanup fixture".into(),
            locale: envbox_core::LocaleProfile {
                locale_name: "en-US".into(),
                ui_language: "en-US".into(),
                region: "US".into(),
            },
            timezone: envbox_core::TimezoneProfile {
                windows_id: "Pacific Standard Time".into(),
                iana_id: "America/Los_Angeles".into(),
            },
            dns: Default::default(),
            environment: Default::default(),
            registry: Default::default(),
            browser: Default::default(),
        };
        let mut container = envbox_core::Container::new("cleanup", profile.id);
        container.id = command.container_id;
        let snapshot = RunSnapshot::new(&container, &profile, command.instance_id).unwrap();
        let cleanup_error = runs.cleanup_failed_start_with_terminate(
            &command,
            &snapshot,
            owned.result,
            owned.job.unwrap(),
            &path,
            "startup failed".into(),
            |_| {
                unsafe { TerminateJobObject(query_only.0, 1) }
                    .map_err(|_| io::Error::last_os_error())
            },
        );
        assert!(cleanup_error
            .to_string()
            .contains("ownership loss journal persistence failed"));
        assert!(runs.running[&command.instance_id]
            .result
            .error
            .as_deref()
            .unwrap()
            .contains("ownership loss journal persistence failed"));
        drop(held);
        let unchanged: RunResult = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            unchanged.error, sealed.error,
            "failed cleanup write must preserve the previous sealed journal"
        );
        assert_eq!(
            runs.control(
                "retry-stop",
                "Stop",
                Some(command.container_id),
                Some(&command)
            )
            .0,
            "Ok"
        );
        assert_eq!(runs.running[&command.instance_id].result.state, "Stopped");
        drop(runs);
        drop(query_only);
        drop(child);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_budget_expiry_refuses_retention_before_path_access() {
        let mut leases = Vec::new();
        let mut fact = record(3).member_runtimes.remove(0);
        fact.module_path = r"\\unreachable.invalid\share\bundle.dll".into();
        let error = retain_bundles(&mut leases, &[fact], std::time::Instant::now()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(leases.is_empty());
    }

    #[test]
    fn bundle_paths_refuse_network_device_relative_and_parent_traversal() {
        for path in [
            r"\\unreachable.invalid\share\bundle.dll",
            r"\\?\UNC\unreachable.invalid\share\bundle.dll",
            r"\\.\PIPE\bundle",
            r"bundle.dll",
            r"C:bundle.dll",
            r"C:\fixture\..\bundle.dll",
        ] {
            let error = check_local_bundle_path(
                std::path::Path::new(path),
                std::time::Instant::now() + std::time::Duration::from_secs(1),
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Other, "{path}");
            assert!(
                error.to_string().contains("local disk")
                    || error
                        .to_string()
                        .contains("absolute without parent traversal")
            );
        }
    }

    #[test]
    fn bundle_paths_refuse_directory_junction_before_canonicalization() {
        use std::os::windows::process::CommandExt;
        let root = std::env::temp_dir().join(format!("aura-lease-junction-{}", Uuid::new_v4()));
        let target = root.join("plain");
        let junction = root.join("redirect");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("bundle.dll"), b"fixture bundle").unwrap();
        let output = std::process::Command::new("cmd.exe")
            .raw_arg(format!(
                "/D /C mklink /J \"{}\" \"{}\"",
                junction.display(),
                target.display()
            ))
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error = check_local_bundle_path(
            &junction.join("bundle.dll"),
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("reparse point"));
        std::fs::remove_dir(&junction).unwrap();
        assert!(target.join("bundle.dll").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_read_that_returns_after_short_deadline_is_not_accepted() {
        struct DelayedRead;
        impl std::io::Read for DelayedRead {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                std::thread::sleep(std::time::Duration::from_millis(25));
                output[0] = 1;
                Ok(1)
            }
        }
        let error = hash_bundle_stream(
            DelayedRead,
            std::time::Instant::now() + std::time::Duration::from_millis(10),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn refresh_persists_unverified_member_loss_before_restart() {
        use std::os::windows::process::CommandExt;
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let root = std::env::temp_dir().join(format!("aura-refresh-loss-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let mut result = record(3);
        result.state = "Running".into();
        result.error = None;
        let command = RunCommand {
            container_id: result.container_id,
            instance_id: result.instance_id,
            application_id: result.application_id,
        };
        let path = root
            .join("containers")
            .join(command.container_id.to_string())
            .join("runs")
            .join(format!("{}.json", command.instance_id));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        atomic_record(&path, &result).unwrap();
        let mut child = Child(
            std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 60",
                ])
                .creation_flags(0x0800_0000)
                .spawn()
                .unwrap(),
        );
        let mut job =
            InstanceJob::create_named_exclusive(&format!("Local\\AuraLossTest-{}", Uuid::new_v4()))
                .unwrap();
        job.assign_pid(child.0.id()).unwrap();
        let targets = Arc::new(Mutex::new(HashSet::new()));
        let mut runs = Runs {
            recovery_error: None,
            generation: "fixture-generation".into(),
            store: store.clone(),
            targets: targets.clone(),
            running: HashMap::new(),
            requests: HashMap::new(),
            control_requests: HashMap::new(),
        };
        runs.running.insert(
            command.instance_id,
            OwnedRun {
                stop_failed: false,
                retry_recovery: false,
                _snapshot: None,
                _recovered_broker: None,
                _bundle_leases: vec![],
                command: command.clone(),
                result,
                session: None,
                job: Some(job),
                members: vec![],
            },
        );
        let mut other = record(3);
        other.state = "Running".into();
        other.error = None;
        let other_command = RunCommand {
            container_id: other.container_id,
            instance_id: other.instance_id,
            application_id: other.application_id,
        };
        let other_path = root
            .join("containers")
            .join(other.container_id.to_string())
            .join("runs")
            .join(format!("{}.json", other.instance_id));
        std::fs::create_dir_all(other_path.parent().unwrap()).unwrap();
        atomic_record(&other_path, &other).unwrap();
        runs.running.insert(
            other.instance_id,
            OwnedRun {
                stop_failed: false,
                retry_recovery: false,
                _snapshot: None,
                _recovered_broker: None,
                _bundle_leases: vec![],
                command: other_command.clone(),
                result: other,
                session: None,
                job: Some(InstanceJob::create().unwrap()),
                members: vec![],
            },
        );
        runs.refresh();
        let sealed: RunResult = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            sealed.state, "TrackingLost",
            "loss must reach the journal immediately"
        );
        assert!(sealed
            .error
            .as_deref()
            .unwrap()
            .contains("member Runtime verification"));
        assert_eq!(
            sealed.known_members,
            runs.running[&command.instance_id].result.known_members
        );
        assert_eq!(runs.query(&other_command).0, "Exited");
        assert_eq!(
            runs.control(
                "stop-other",
                "Stop",
                Some(other_command.container_id),
                Some(&other_command)
            )
            .0,
            "Ok"
        );
        let sealed_other: RunResult =
            serde_json::from_slice(&std::fs::read(&other_path).unwrap()).unwrap();
        assert_eq!(
            sealed_other.state, "Exited",
            "A loss must not overwrite B's terminal journal"
        );
        drop(runs);
        let mut restarted = Runs::new(store, targets, "next-generation".into());
        assert_eq!(
            restarted.running[&command.instance_id].result.state,
            "TrackingLost"
        );
        assert!(restarted.running[&command.instance_id].job.is_none());
        assert_eq!(
            restarted
                .control(
                    "stop-unknown",
                    "Stop",
                    Some(command.container_id),
                    Some(&command)
                )
                .0,
            "NotControlled"
        );
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "restart must not terminate an unknown member"
        );
        drop(restarted);
        drop(child);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loss_journal_write_failure_is_visible_and_keeps_the_last_sealed_record() {
        let root = std::env::temp_dir().join(format!("aura-loss-write-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("run.json");
        let mut result = record(3);
        result.state = "Running".into();
        result.error = None;
        atomic_record(&path, &result).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        persist_tracking_loss(
            &path,
            &mut result,
            "member generation changed".into(),
            false,
        );
        assert_eq!(result.state, "TrackingLost");
        let reason = result.error.as_deref().unwrap();
        assert!(reason.contains("member generation changed"));
        assert!(reason.contains("ownership loss journal persistence failed"));
        drop(held);
        let sealed: RunResult = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            sealed.state, "Running",
            "failed write must not corrupt the sealed record"
        );
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            1,
            "failed atomic writes must clean their temporary files"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn member_failure_during_stop_preserves_stopping_and_persists_the_reason() {
        let root = std::env::temp_dir().join(format!("aura-stopping-loss-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("run.json");
        let mut result = record(3);
        result.state = "Stopping".into();
        persist_tracking_loss(
            &path,
            &mut result,
            "Runtime bundle retention failed".into(),
            true,
        );
        let sealed: RunResult = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(sealed.state, "Stopping");
        assert_eq!(sealed.error, result.error);
        assert!(sealed
            .error
            .as_deref()
            .unwrap()
            .contains("Runtime bundle retention failed"));
        std::fs::remove_dir_all(root).unwrap();
    }
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
                environment_facts: None,
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

    #[test]
    fn missing_job_classification_requires_every_sealed_generation_to_be_gone() {
        let identities = vec![
            ProcessIdentity {
                pid: 41,
                creation_time: 410,
            },
            ProcessIdentity {
                pid: 42,
                creation_time: 420,
            },
        ];
        assert!(process_generation_is_absent(&identities, |pid| {
            Err(io::Error::from_raw_os_error(if pid == 41 {
                87
            } else {
                1168
            }))
        })
        .unwrap());
        assert!(process_generation_is_absent(&identities, |pid| {
            Ok(ManagedTarget {
                pid,
                creation_time: if pid == 41 { 999 } else { 420 },
            })
        })
        .is_ok_and(|absent| !absent));
        assert!(process_generation_is_absent(&identities, |pid| {
            if pid == 41 {
                Err(io::Error::from_raw_os_error(5))
            } else {
                Err(io::Error::from_raw_os_error(87))
            }
        })
        .is_err());

        let mut incomplete = record(3);
        incomplete.member_runtimes.pop();
        assert!(!sealed_members_are_absent(&incomplete).unwrap());
    }

    #[test]
    fn active_bundle_lease_blocks_delete_until_run_releases_it() {
        let root = std::env::temp_dir().join(format!("aura-bundle-lease-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("envbox-runtime64.dll");
        std::fs::write(&path, b"fixture bundle").unwrap();
        use sha2::Digest;
        let expected_hash = format!("{:x}", sha2::Sha256::digest(b"fixture bundle"));
        let mut leases = Vec::new();
        retain_bundles(
            &mut leases,
            &[MemberRuntimeIdentity {
                pid: 1,
                creation_time: 1,
                module_path: path.clone(),
                module_sha256: expected_hash.clone(),
                config_sha256: String::new(),
                runtime_version: String::new(),
                environment_facts: None,
            }],
            std::time::Instant::now() + std::time::Duration::from_secs(4),
        )
        .unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        leases.clear();
        std::fs::write(&path, b"tampered bundle").unwrap();
        let mut rejected = Vec::new();
        assert!(retain_bundles(
            &mut rejected,
            &[MemberRuntimeIdentity {
                pid: 1,
                creation_time: 1,
                module_path: path.clone(),
                module_sha256: expected_hash,
                config_sha256: String::new(),
                runtime_version: String::new(),
                environment_facts: None,
            }],
            std::time::Instant::now() + std::time::Duration::from_secs(4),
        )
        .is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
