# Trusted service bootstrap native fixture

This fixture compiles the production `service_bootstrap.cpp` validator and runs
each immutable-flag case in a separate x64/x86 process. It also loads the actual
Runtime DLL with a complete ENV Profile: legacy mode can use ENV fallback,
whereas trusted mode rejects an ordinary user-owned server without falling back.
The pipe test opens the exact production client access mask and checks that
another server instance is refused with `ERROR_ACCESS_DENIED`.

Build both architectures with MSVC/CMake:

```powershell
cmake -S tools/envbox-service-bootstrap-fixture -B target/service-bootstrap-fixture64 -A x64
cmake --build target/service-bootstrap-fixture64 --config Release
cmake -S tools/envbox-service-bootstrap-fixture -B target/service-bootstrap-fixture32 -A Win32
cmake --build target/service-bootstrap-fixture32 --config Release
./tools/envbox-service-bootstrap-fixture/run.ps1 -RuntimeDirectory <frozen-runtime-pair>
```

The runner starts a fresh WMI controller, requires zero loaded Runtime modules,
clears inherited ENVBOX variables, records Runtime hashes and all raw results in
a unique `target/service-bootstrap-*` directory. It neither installs nor starts
a service or driver. There is no simulated positive LocalSystem/service-SID
claim: that positive path requires an actual service environment and remains
unverified here. The ACL test demonstrates the requested access bits; a pipe
owned by the same user is not a trust boundary against that owner rewriting its
DACL. Actual service authentication uses the OS server process's primary token,
session zero, LocalSystem user and enabled dedicated service SID.

Verified on 2026-10-06: the fresh WMI controller PID 23348 had zero loaded
Runtime modules; all 24 x64/x86 cases passed. Raw output is preserved under
`target/service-bootstrap-3e9ffe14c3fc4fd69cbac3bf5d7f80df/`. Both architectures
loaded the legacy ENV Profile, while trusted fake-server and invalid-flag cases
returned `ERROR_DLL_INIT_FAILED` (1114). Fixture builds use `/W4 /WX`; both
fixture and Runtime Release builds passed. The frozen Runtime pair is
`target/service-bootstrap-runtime/`:

| Architecture | SHA-256 |
| --- | --- |
| x64 | `1454D005485AF5B1C66158B5CF3489515A1BDAF35238C11F9DCB8E9C2C59516B` |
| x86 | `6BB02A675E6C978E1E097D10DC13F70D537F92C6FC32A2733D85CC2220C4F3EE` |

An initial runner reached all native cases but stalled serializing PowerShell 5
`Get-Content` extended string properties. Its controller was terminated and
raw logs remain under `target/service-bootstrap-4a89dc59ade74f6890335902f81abac0/`.
The runner now reads plain strings using `File.ReadAllText`; the final run above
exited zero. This was a fixture reporting fault, not a claimed successful run.
