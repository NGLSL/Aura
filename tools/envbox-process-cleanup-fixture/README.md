# Process creation cleanup regression fixture

This standalone Windows fixture exercises public `CreateProcessW` calls in a
host process and an actually injected Runtime process. It alternates 33 missing
image failures with successful child creation, verifies the Windows error code,
checks that a caller-owned event remains valid, waits for every child, and checks
the process handle count before and after the measured cycles. Injected children
must themselves load the matching Runtime. Audit data is confined to a `.config`
directory beside the fixture executable.

Build using a Visual Studio developer environment and the built Detours library:

```powershell
cmake -S tools/envbox-process-cleanup-fixture -B target/cleanup-fixture64 -A x64 -DDETOURS_ROOT=D:/Tools/Detours
cmake --build target/cleanup-fixture64 --config Release
target/cleanup-fixture64/Release/envbox-process-cleanup-fixture.exe --check
target/cleanup-fixture64/Release/envbox-process-cleanup-fixture.exe --inject D:/Project/Aura/target/dns-runtime/Release/envbox-runtime64.dll
```

For x86, configure a separate directory with `-A Win32` and supply
`envbox-runtime32.dll`. Use a genuinely uninjected launcher for the Host control;
an Aura-injected terminal can propagate its own Runtime into the fixture. The
fixture rejects a Host control if it detects a Runtime module already loaded.

The default checks do not force Detours injection, Job assignment, or thread
resume failures. Passing them is not evidence that those fault paths were
reproduced. The previous implementation also passed the missing-image regression;
the double-close fix is grounded in the pinned Detours ownership contract.

## Controlled native failures (test-only)

The separate `cleanup-fault64.dll` / `cleanup-fault32.dll` is a fixture component.
It must never enter a product Runtime bundle or installer. The driver imports it
before the matching Runtime into its own child and calls the public Windows
process-creation API. Fault hooks affect only this child process and its explicitly
named fixture executable.

```powershell
# Mode 1: terminate this fixture's suspended child before Detours can inject it;
# its helper attempt receives a native missing-image error.
target/cleanup-fixture64/Release/envbox-process-cleanup-fixture.exe --fault-inject D:/Project/Aura/target/dns-runtime/Release/envbox-runtime64.dll D:/Project/Aura/target/cleanup-fixture64/Release/cleanup-fault64.dll 1

# Mode 2: call the native ResumeThread API with a query-only duplicated handle,
# producing a real access-denied error while the owned child stays suspended.
target/cleanup-fixture64/Release/envbox-process-cleanup-fixture.exe --fault-inject D:/Project/Aura/target/dns-runtime/Release/envbox-runtime64.dll D:/Project/Aura/target/cleanup-fixture64/Release/cleanup-fault64.dll 2
```

Each mode checks the returned error against the native fault error, waits on an
owned duplicate of the exact child process handle, records each Process/Thread
close and termination, then alternates 32 failures with successful child runs.
Fallback cleanup marks a failure; it cannot be reported as product cleanup.

The DLL also contains an invalid-Job native fault hook (mode 3) for a future
isolated CLI driver. The current driver does not execute that scenario.

**Validation status (2026-10-04):** both fixture architectures and fault DLLs build.
The original ordinary Host and injected checks ran successfully on both
architectures. The expanded fault driver has **not run successfully**: fresh
uninjected WMI/PowerShell and the existing suspended helper returned native
`CreateProcess` error 5 when starting the rebuilt executable, before its tests
ran. No host security settings were changed to work around this restriction.
Fault-path coverage remains unverified until these commands run in a suitable
isolated Windows environment.
