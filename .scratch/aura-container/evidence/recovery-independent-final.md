# Recovery and Runtime bundle independent slice

Date: 2026-10-06
Related: [12 Supervisor crash recovery](../issues/12-ticket.md), [13 active Runtime bundle retention](../issues/13-ticket.md)
Baseline: `a5baed8`

This slice covers the parts that can be implemented and exercised on the
current Windows host without a VM, driver loading, or an OS reboot.

## Missing Job and unknown tree behavior

When a persisted Run cannot reopen its named Job, the Supervisor now keeps the
Run in `TrackingLost`. It does not infer `Exited` from the absence of the
sealed PID generations. A Job handle disappearing does not provide a durable
complete descendant list, and a process created outside the Job or after the
last journal seal could still be alive. The record is persisted with an
explicit reason:

```text
named tracking Job no longer exists; sealed generations are absent but the complete process tree cannot be proven
```

The diagnostic check requires schema 3, a non-empty unique member set, and a
one-to-one member Runtime proof before it calls the sealed generations absent.
PID reuse is treated as absence of the old generation. Direct process
inspection errors other than the documented not-found/invalid-PID forms remain
unknown and keep the record lost. Older schema 2 or incomplete member evidence
cannot reach the diagnostic classification.

The record is written as `TrackingLost` when initial recovery fails and again
when a retry fails. This prevents a restarted Supervisor from silently leaving
an old `Running` journal after it has learned that control cannot be proved.
The existing safe terminal transition remains narrower: a reopened Job whose
live member count is confirmed as zero becomes `Exited`; a missing Job never
does.

The native fixture [recovery_terminal.rs](../../../crates/envbox-supervisor/tests/recovery_terminal.rs)
creates a real valid snapshot and schema 3 journal, uses a valid scoped but
nonexistent Job name, and performs a real Windows process-generation query for
an absent PID. A fresh uninjected Supervisor serves the journal and returns
`TrackingLost` with the complete-tree reason; the persisted JSON is checked
after the server exits. This is a negative safety test, not proof that every
escaped descendant can be discovered after Job loss.

## Runtime bundle retention

Each active or recovered Run keeps a read handle for every authenticated member
Runtime image. The handle deliberately requests `FILE_SHARE_READ` only. A
concurrent write or delete therefore fails with the Windows sharing rule while
the Run remains under management. Lease acquisition rejects reparse points,
opens the image before taking its hash, compares Windows volume/file identity
before and after the open, and hashes the bounded stable handle contents
against the authenticated member SHA-256. Repeated observations of an already
leased path still verify its file identity and expected SHA-256; they cannot
silently accept a replaced file. Newly observed cross-architecture members
extend the same lease set. Leases are released only after the Job reports zero
active processes, and a failed recovery keeps leases while the original Run is
still `TrackingLost` and its images remain available.

The launcher already stages content-addressed paired Runtime files outside the
installation directory and never overwrites a conflicting cache directory.
This Supervisor lease closes the remaining in-process overwrite/delete race;
it does not claim to control an arbitrary external cleanup tool after the
Supervisor itself has crashed. Installers must continue to preserve the
staged cache as a separate upgrade contract.

## Verification

On the current fresh uninjected Windows host:

```text
cargo +1.99.0 test -p envbox-supervisor --lib --test recovery_terminal --locked -- --nocapture
5 unit tests passed
1 native missing-Job fixture passed
```

The unit coverage includes generation mismatch/not-found/error classification,
incomplete member evidence refusal, an actual file lease that rejects both
write-open and delete until the lease is dropped, and a post-release tamper
whose SHA-256 is rejected on reacquisition. The native fixture confirms the
persisted `TrackingLost` result through the Supervisor management pipe.

This evidence does not close ticket 12 or 13. OS reboot, real unknown console
companions, installer upgrade execution, Supervisor protocol version migration,
and driver/VM recovery remain separate acceptance gates.
