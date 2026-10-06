# AuraPolicyService source prototype

This crate provides the real Windows SCM dispatcher for `AuraPolicyService`, a
local management pipe and a separately callable qualification engine. It does
not install/start a service, load a driver, change privileges, change the host
network, or enable the core Container capability.

The default SCM request handler rejects every launch/status/stop operation with
`kernel backend is not qualified for Container launch`. No environment variable,
request field, command-line flag or profile can open this gate. `PrototypeEngine`
is the source seam for later isolated service/driver qualification, not a claim
that an installed production control plane already exists.

## Authenticated management boundary

- The service must have a session-zero LocalSystem primary token and the exact
  enabled, non-deny-only `AuraPolicyService` SID. It must not impersonate when
  accessing the driver.
- The fixed local pipe is `\\.\pipe\AuraPolicyService.v1`. Its first instance is
  exclusive, remote clients are rejected, the mandatory label rejects low
  integrity writes, and Authenticated Users receive `0x0012019b`. This mask
  excludes `FILE_CREATE_PIPE_INSTANCE`; generic write must not replace it.
- The server reads one message of at most 64 KiB within a five-second deadline.
  Immediately after connection it first pins the actual client process and
  primary-token identity, approves its protected manager image, and rejects
  Runtime-loaded peers. Unapproved silent connections are disconnected before
  consuming that read deadline. After reading, full pipe impersonation checks
  still establish the message security context and match the retained preflight
  PID/creation time, SID, authentication LUID and session.
  Launch fields reference an existing immutable snapshot by Container/Profile,
  Instance/Snapshot UUID, configuration ID and content digest. Client-provided
  full Profiles are rejected. PID, token, SID, driver handle, Runtime
  path, privileged job name and capability flags are not accepted.
- Peer identity is derived from the real pipe client PID and retained process
  handle, then compared with the pipe impersonation token: user SID,
  authentication LUID, session, groups, restrictions and integrity. Ordinary
  non-elevated interactive medium-integrity clients are required; AppContainer,
  network-logon, low/high integrity and session-zero peers are rejected. The
  primary token is duplicated from this independently authenticated process.
- `authenticate_server` lets clients pin the actual server process and validate
  its LocalSystem/service-SID identity before sending launch material. An actual
  local spoof pipe is covered by a negative native API test. A complete GUI/CLI
  management client is not connected yet.

## Qualification engine sequence

`PrototypeEngine::prepare` validates service identity, holds protected installed
bundle leases, and opens the driver session before a target can be created. It
uses the directory of the service executable, containing an approved `aura.exe`
and/or `envbox.exe` management image plus
`envbox-runtime64.dll` and `envbox-runtime32.dll`; no inherited Runtime override
or per-user staging directory participates. Effective ACLs must have privileged
owners and no ordinary-user write/delete/owner/DACL grant, unsupported ACEs fail
closed, and ancestors cannot be reparse points. Leases deny write/delete sharing.

The connected management process must use the approved installed manager image;
its path, actual file identity and SHA-256 match the leased image. A target cannot
be the installed management bundle, even via a hardlink. Target and ancestor
leases prevent replacement between validation and creation.
Input executable/working-directory paths must use local fixed drives without
raw dot/parent components, trailing-dot/space aliases, alternate data streams,
DOS device names, device namespace, remote share or reparse components. Ordinary
Windows DOS separators, including `/`, follow the single shared launcher guard. A root
to leaf lease walk happens before privileged canonicalization, preventing a
request from sending LocalSystem filesystem/credential access to a remote path.
Management processes with a loaded `envbox-runtime32.dll`/`envbox-runtime64.dll`
are rejected, failed module inspection fails closed, and actual retained-process
membership in any owned service Job rejects delegation. This is the documented
Runtime module identity check, not a claim of detecting arbitrary renamed or
hostile injected modules.

The service obtains LocalAppData through `SHGetKnownFolderPath` with the
independently authenticated user token. KnownFolder resolution, no-reparse path
leases and ConfigStore reads happen under that user's impersonation. It never
uses `ConfigStore::default_root`, `ENVBOX_CONFIG_ROOT` or System's LocalAppData,
and never prepares or writes snapshots. Required documents are bounded to one
MiB, then existing Container/Profile identity bindings, Snapshot/Instance IDs,
configuration ID, content digest and `RunSnapshot::validate` are checked.
The current live Profile must exist and match the Container's Profile ID; its
edited contents do not invalidate an already-prepared frozen effective Profile.

The source launch path calls `start_session_as_user` with this service-read
snapshot's effective Profile, the independently authenticated ordinary-user
token and an opaque `TrustedRuntimeBundle` capability. Arbitrary bare Runtime
paths cannot substitute for its protected stable leases. The service is the
actual process creator. Its callback applies the 136-byte kernel wire against
the actual retained target handle **before the first ResumeThread**. After that,
the existing verified entry gate waits for Runtime identity and capability ACK
before allowing application entry. Full Job/Broker/Session ownership remains in
the service registry; returning a UUID does not transfer process ownership to a
GUI or Supervisor.

Container UUID and Profile UUID are distinct. The driver digest includes the
canonical authoritative stored snapshot and independent Host/Deny envelope;
this does not claim Core supports Container mode. Run ownership binds user SID, authentication LUID and
session. Requests are bounded and their success/failure is retained for replay;
changing payload or owner under the same request UUID fails. Reusing a container
under another owner or an already-used instance UUID fails. Status queries the
actual owned Job, and Stop terminates that Job while retaining its tracking
handle. Failed Stop remains an error until an explicit successful retry.

Pipe disconnect/process exit is checked before binding. The source launch uses a
ten-second cooperative check before bind; synchronous filesystem/kernel API calls
cannot be interrupted by this check. This is not a hard end-to-end wall-clock
deadline or a qualified service shutdown guarantee.

## Verification and limits

`cargo check -p envbox-policy-service` and
`cargo test -p envbox-policy-service --lib` compile the real Windows API paths and
cover wire offsets, independent container identity, authoritative snapshot
reference/digest checks, framing, invalid IDs, unauthorized driver-session
preparation and real named-pipe server spoof rejection. Isolated on-disk fixtures
read through a native current-user token impersonation cover missing/tampered
snapshots, wrong Container/Profile/digest and live-Profile edit freeze behavior.
These are real fixture reads, not System service positive qualification.
An actual local unauthorized pipe connection sends no payload and is rejected
before any read; the targeted test recorded 0 ms against a 2-second bound, while
the authorized read deadline remains 5 seconds.

Actual LocalSystem/service-SID execution, positive ordinary-user pipe
authentication, installed bundle qualification, user desktop/profile behavior,
SCM stop/restart, restart/recovery of retained kernel bindings, clone paths,
driver/WFP operation and load/Verifier testing are not proven here. The manager
client, persistent service journal, deployment/installation and production
capability enablement remain separate work. Existing driver qualification
requirements continue to block loading on the host.

Primary API references:

- [Processes in the client security context](https://learn.microsoft.com/en-us/windows/win32/secauthz/processes-in-the-client-security-context)
- [Named pipe security and access rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)
- [StartServiceCtrlDispatcherW](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-startservicectrldispatcherw)
