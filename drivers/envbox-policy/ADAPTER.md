# Source-only WDM identity adapter

`adapter.c` implements a highest-level WDM control device and kernel identity
adapter around the shared C core. It has been compiled and linked as an x64
native SYS using cached WDK/SDK packages. **It has never been loaded.** No service,
driver registration, signing, catalog generation, boot-policy change or kernel
runtime test was performed. Container remains unavailable.

## Controller authority

The service name is fixed to `AuraPolicyService`. Its SID was calculated with
read-only `sc.exe showsid`; `service_identity.h` contains the canonical text and
32 binary bytes. The independent build verifies both against Windows SID
conversion and read-only `sc.exe showsid`. The host fixture parses the actual
SDDL and requires exactly one allow ACE for that SID.

The named device uses `IoCreateDeviceSecure` with
`D:P(A;;GA;;;S-1-5-80-3820527054-1736232212-1554131609-2082016452-471345742)`
and `FILE_DEVICE_SECURE_OPEN`. There is no administrator, Everyone or general
LocalSystem grant. A device ACL alone is insufficient: Create and every IOCTL
also require primary-token LocalSystem **and** the enabled dedicated service
SID, reject deny-only membership and any thread impersonation, require UserMode
and the exact requestor/current-process context, and reject device path suffixes.
The same live `PEPROCESS` and OS creation time own an exclusive FO session.
These checks survive a broader registry override of the device's default ACL;
the override has not been exercised. The future service must also authenticate
its user clients and immutable launch requests; that service does not exist yet.
The wire owner is the canonical binary service SID, not a user-selected UUID.

Each successful connection gets a kernel-generated increasing generation.
`IOCTL_EB_POLICY_SESSION` requires zero input and exactly 8 output bytes (LE64).
`IOCTL_EB_POLICY_APPLY` requires exactly 136 input bytes and no output. Both use
METHOD_BUFFERED and require read/write access respectively, not FILE_ANY_ACCESS.
Unsupported IOCTLs and malformed policy packets never change bindings.

## Actual process references

Creation callbacks use the actual current creating process object, not the
spoofable ParentProcessId, a wire PID, environment or image-path ownership. Only
the currently authenticated controller's creations can be registered Pending.
The callback rechecks controller token, rejects subsystem/no-image cases, checks
creation time, obtains a real process-object reference and stores it beside the
same occupied core slot. Capacity exhaustion or channel loss before registration
sets CreationStatus to an error. Ordinary unrelated Host creation is untouched.
An already registered member's new child is explicitly rejected **only on
creation paths that actually invoke the registered process callback**. This is
not a guarantee covering all possible descendants. Child inheritance and
service/broker delegation are unsupported.

Binding resolves the service's supplied handle with `ObReferenceObjectByHandle`,
`UserMode`, `*PsProcessType` and QUERY_LIMITED_INFORMATION | SUSPEND_RESUME access.
Kernel/negative pseudo handles and nonrepresentable WOW64 values are rejected.
SDK access/group constants absent from this cached ntifs.h are namespaced in
`adapter.h`; the host compilation asserts them against actual SDK definitions.
An exiting target is refused. The referenced object and creation time must match
an already callback-established slot: a handle cannot adopt an existing Host.
Only then does the immutable core bind/revoke run. Every temporary handle-derived
reference is released, including failure paths.

The OS exit callback removes a binding only for that retained object and creation
time, then releases its retained reference outside the core lock. Controller
exit disconnects the exact session. FO Cleanup disconnects without removing
process bindings, and Close releases the FO's controller reference. A separate
bounded controller-reference table preserves disconnected controller identity
until its real exit; disconnected controllers cannot silently create Host
processes. Verified reconnection requires a new generation and the same service
identity. Metadata/strategy recovery and normal user app support are unverified.

## Execution and lifecycle limits

Creation registration is an identity mechanism, **not a verified launch gate**.
The future dedicated service must create suspended, bind, attach other required
resources, independently confirm the startup contract, then Resume. No actual
service/gate path has been tested. There is no WFP filtering, so Pending/Deny in
the table does not presently block network traffic or prove a first-packet
boundary. Host/Deny classification is currently exercised only in the host core
fixture. Inbound/listening, Allowlist/TTL, child inheritance and storage filters
remain unsupported. IPv6 is deferred independently of this source work.

Native process cloning and PSS VA cloning are a separate mandatory qualification
gate. The documented legacy process-notification behavior excludes clone
creation; this adapter has not demonstrated notification, trusted ownership or
actual rejection for those paths. An unknown object receiving the core's normal
Host classification cannot establish complete descendant protection. Do not
change all unknown Host processes to Deny as a substitute: that would affect
unrelated host applications. Do not introduce undocumented Native hooks.

Before any driver loading qualification or Container/Strong enablement, isolated
Windows tests must demonstrate that Native clone and PSS VA clone attempts from
protected members either acquire independently verified ownership before any
protected execution/network operation, or are actually rejected. An Unsupported
label or a source-only callback check does not satisfy this gate. Until that
evidence and enforcement exist, driver loading and Container/Strong enablement
remain prohibited, alongside the other lifecycle and startup requirements.
There is no claim of a currently loaded-driver vulnerability: this adapter has
never been loaded.

Token queries and dispatch are PASSIVE_LEVEL. EX_PUSH_LOCK in a critical region
serializes core calls and reference tables; token queries and user-handle
resolution occur outside that lock, then session/generation are checked again.
Process callbacks run at PASSIVE_LEVEL. The source has no asynchronous work or
queued IRPs. A future DISPATCH_LEVEL WFP callout cannot use this passive lock
unchanged; it needs a separately reviewed resident classification/read-lifetime
design.

The driver deliberately sets `DriverUnload = NULL`. A void WDM unload callback
cannot reject unload after protection exists, and there is no verified safe
drain/recovery protocol. This prototype is **not eligible for loading**, including
on the current host. A formal backend must implement qualified stop/drain,
callback rundown, reference cleanup, recovery and safe update/unload before
any approved VM runtime experiment. DriverEntry performs all fallible device
and symlink work before callback registration; failed registration deletes
those objects, and no fallible setup remains after a callback can run.

## Build and proof

```powershell
./tools/envbox-policy-fixture/build-driver.ps1
./tools/envbox-policy-fixture/run.ps1
```

The independent driver build performs `/kernel /W4 /WX /Zl /X` compilation,
links `/driver /subsystem:native /nodefaultlib /integritycheck`, verifies x64
PE32+, native subsystem and FORCE_INTEGRITY, and permits only ntoskrnl/HAL imports.
Current linked output imports only `ntoskrnl.exe`. Logs and SYS hash are in
`target/envbox-policy-driver/result.json`. Build success proves source/DDI/link
compatibility only. Host tests prove wire/state/SID/SDDL and C/C++ linkage,
not kernel authentication or filtering. Tickets 22/23 remain unaccepted.

Primary sources used for these contracts:

- [IoCreateDeviceSecure](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdmsec/nf-wdmsec-wdmlibiocreatedevicesecure)
- [SeQueryInformationToken: PASSIVE_LEVEL and caller-owned paged allocations](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-sequeryinformationtoken)
- [PsReferenceImpersonationToken: reference lifetime and no-token result](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-psreferenceimpersonationtoken)
- [ObReferenceObjectByHandle: type/access checking and UserMode](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-obreferenceobjectbyhandle)
- [Process notification registration and removal](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/nf-ntddk-pssetcreateprocessnotifyroutineex)
- [Process callback IRQL](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/nc-ntddk-pcreate_process_notify_routine_ex)
- [PS_CREATE_NOTIFY_INFO: parent versus creator, image object and rejection status](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/ns-ntddk-_ps_create_notify_info)
- [Legacy CreateProcessNotifyEx: clone notification exclusion](https://learn.microsoft.com/en-us/previous-versions/ff542860%28v%3Dvs.85%29)

The actual cached WDK headers and libraries were also inspected. No undocumented
EPROCESS offsets, pattern scans or syscall interception are used.
