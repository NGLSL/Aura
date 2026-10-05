# Supervisor low-integrity management fixture

This fixture exercises ticket 09's local integrity boundary without changing
the host pipe ACL or requiring an elevated helper. The Rust channel test starts
one Supervisor in the current process and records its exact endpoint. The
native launcher duplicates its own same-user token with `CreateRestrictedToken`,
sets `TokenIntegrityLevel` to the Windows low RID (`0x1000`), and launches a
real low-integrity probe with `CreateProcessAsUserW`.

The probe receives the medium/high endpoint as an explicit argument. It never
uses the production endpoint derivation, so a low-integrity endpoint being
absent cannot make this test pass. It records its PID, same-user SID, low
integrity RID, and the actual `CreateFileW` result for that endpoint. On the
current Windows host, the default named-pipe DACL plus mandatory integrity
check rejects the direct open with `ERROR_ACCESS_DENIED` before a request is
delivered. If a future pipe policy permits the open, the same probe sends a
real `Ping` and the test requires `AuthenticationDenied` from the server.
Only Win32 error 5 is classified as the expected `denied` result. Other
`CreateFileW` errors are emitted as `error` and make the native helper fail.

After either rejection path, the original medium/high Rust manager pings the
same Supervisor with the original generation. The test requires `Ok` and an
unchanged generation, demonstrating that the negative attempt did not alter
management state.

The runner also launches the fixture against a freshly generated nonexistent
pipe name. That control must report `pipe_open=error`, error code 2, and a
nonzero process exit; otherwise the fixture would be able to pass on a simple
`NotFound`.

Build and run from the repository root:

```powershell
./tools/envbox-management-fixture/run.ps1
```

The script requires a fresh uninjected host process and writes the host check,
test output, and exit marker below `target/`. It owns only its temporary child
processes and closes/terminates them on the bounded timeout path.
