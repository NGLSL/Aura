# Kernel policy core

This directory contains the allocation-free C state machine intended to be
compiled into the future kernel identity/WFP adapter. The host fixture compiles
**this same source**, rather than a separate model. This directory also has a
source-only WDM identity adapter (`adapter.c`), described in [ADAPTER.md](ADAPTER.md).
The adapter has been compiled and linked, never loaded or exercised in kernel
mode. There is no WFP callout or Windows service. It must not enable Container
mode or advertise functioning network isolation.

The adapter's child-creation rejection covers only paths that actually invoke
its process callback. Native clone and PSS VA clone coverage is unverified and
is a separate mandatory **driver-loading and Container/Strong qualification
gate**. Isolated tests must prove trustworthy clone ownership before protected
execution/network activity, or actual clone rejection. Merely recording
Unsupported does not qualify these paths; unknown Host classification is not a
complete descendant guarantee. Until those mechanisms and evidence exist,
loading and Container/Strong enablement remain prohibited. See
[the adapter qualification contract](ADAPTER.md#execution-and-lifecycle-limits).

The current scope is IPv4 outbound TCP/UDP, including loopback, with immutable
Host or Deny policy. IPv6 is deferred. Unsupported families, protocols and
directions return a distinct unsupported result: adapters must reject the
request, not translate it into Host. Allowlist, DNS TTL binding, child
inheritance and inbound/listening policy are not implemented.

## Trust and launch contract

The controller must eventually be a dedicated system service with a dedicated
service SID. A device ACL must grant only the necessary rights to that service
SID (and the OS identity necessary for installation/lifecycle). Authentication
must validate the actual connection/token identity; neither every administrator
nor a per-user Supervisor is an authorized policy controller by default. The
service must separately authenticate user-management requests and their owner.
The 32-byte owner identifier in this core is an adapter-established identity,
not a client-selected secret or the complete service authorization mechanism.

The adapter must own a retained reference to each process object. Before
releasing the launch gate, it calls `eb_mark_pending` with the object identity
and OS creation time. Failure, including capacity exhaustion, means **do not
release the target**. An ordinary unregistered host process is Host; a pending
protected process is Deny. This distinction cannot be established by an
application's environment, PID, module marker or Runtime assertion.

For a binding request, the adapter must:

1. Copy the fixed wire packet into kernel-owned memory and decode it.
2. Verify the authenticated controller generation and owner with `eb_authorize`.
3. Resolve `process_handle` in the requestor's handle context with
   `ObReferenceObjectByHandle` using `UserMode`, `*PsProcessType` and the required
   rights. For WOW64, enforce handle representability; never truncate arbitrary
   64-bit values or interpret an unchecked handle as a kernel address.
4. Independently verify that the caller may bind this retained object and that
   its creation time matches the launch transaction. The wire has no PID or
   caller-supplied process creation time.
5. Under the same serialization contract, call `eb_apply` with the verified
   object identity; publish the binding and release the launch gate only after
   success. Resolve the handle again for each request, including idempotent
   retries. Release temporary object references on every path.

The source-only adapter now implements the service token/connection checks,
UserMode handle resolution and callback-established process ownership. Its
kernel/API behavior, callback ordering and privilege negative tests remain
unverified. It has no proven launch gate or startup integration; those contracts
remain to be implemented and verified in an isolated environment.
Object pointer keys must stay inside kernel memory and must not enter audit
or user responses. This core never takes ownership of references itself.

## State and lifetime

The caller owns a bounded 64-slot table, allocates it from appropriately resident
kernel memory (not a large kernel-stack local), and serializes **every** call,
including classification, with its own suitable lock. The core has no internal
lock, allocation, OS call or background work. The adapter must define legal
IRQLs, rundown and callback ordering before using it from WFP.

Bindings contain immutable Container UUID, Instance UUID, configuration digest
and policy. Equal binds are idempotent; changes are conflicts. Revocation
retains those identities and changes the slot to a terminal Pending/Deny state;
it cannot be rebound or downgraded while that process exists. Only an OS exit
notification with the exact retained process key **and creation time** removes
the slot. A stale exit cannot clear a reused identity.

Disconnect preserves all bindings, including Deny and revoked slots, and blocks
new pending/bind/revoke requests. Reconnection requires explicit independently
verified authentication, the same owner, and a strictly increasing nonzero
generation. Existing immutable bindings can then be retried, not reassigned.
An active controller cannot be replaced without an explicit disconnect.

## Version 1 wire format

All numbers are unsigned little endian. Total length is exactly 136 bytes;
the decoder rejects trailing bytes, unsupported versions, zero identities,
unknown operations/policy and nonzero reserved bytes without changing its output.
There are no wire pointers, variable strings, payload arrays or implicit packing.
The public byte-array view has compile-time assertions for every field offset
and the total size; the decoder reads individual LE bytes, not native structs.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | Version = 1 |
| 4 | 4 | Length = 136 |
| 8 | 4 | Operation: bind = 1, revoke = 2 |
| 12 | 4 | Policy: Host = 0, Deny = 1; revoke requires Deny |
| 16 | 8 | Authenticated controller generation |
| 24 | 8 | Requestor process handle, for adapter validation |
| 32 | 32 | Authenticated owner identity |
| 64 | 16 | Container UUID |
| 80 | 16 | Instance UUID |
| 96 | 32 | Immutable configuration digest |
| 128 | 8 | Reserved, zero |

## Validation boundary

Run `tools/envbox-policy-fixture/run.ps1`. The Windows MSVC x64/x86 fixture uses
`/W4 /WX` and directly compiles `policy.c`. It verifies malformed messages,
ownership/generation rejection, idempotence and immutable conflicts, pending
Deny, disconnect/reconnect, terminal revocation, exact exit/reuse cleanup,
bounded capacity and supported/unsupported decisions. It never loads a driver,
filters traffic or opens a network endpoint. Host results do not satisfy the
real kernel/WFP acceptance of tickets 22/23.

`tools/envbox-policy-fixture/build-kernel-object.ps1` additionally compiles the
same C source as an x64 `/kernel /W4 /WX /Zl /X` object with cached WDK/SDK
10.0.26100.6584 packages from the existing empty-driver build. It force-includes
`ntddk.h`, checks the actual compiler exit code and rejects undefined external
symbols. It does not fetch packages or require a global WDK installation. This
proves kernel compiler/header compatibility only, not driver linking, loading,
WFP behavior, controller authentication or safe unload.
