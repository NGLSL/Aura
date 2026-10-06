# Shared kernel policy host fixture

Run from the repository root:

```powershell
./tools/envbox-policy-fixture/run.ps1
```

The script finds CMake at the default local VS Build Tools path; override it
with `-CMakePath` if needed. It rejects an injected controller process, builds
and runs MSVC x64 and Win32 executables, checks actual binary exit codes, and
writes logs per architecture and `target/envbox-policy-fixture-results.json`
with binary hashes. No driver/service is installed and no network or trust
settings change. See the [shared core contract](../../drivers/envbox-policy/README.md)
for adapter requirements and unverified kernel behavior.
The fixture additionally parses the service SID/SDDL and checks C++ consumers
link against the actual C object in both architectures.

Optional x64 kernel object compilation, using the existing local package cache:

```powershell
./tools/envbox-policy-fixture/build-kernel-object.ps1
```

This force-includes cached WDK headers and emits only a C object. No SYS is linked
or loaded, and no package restore or system installation is performed.

Independent source-only WDM SYS build and PE/import checks:

```powershell
./tools/envbox-policy-fixture/build-driver.ps1
```

This uses the same cached WDK, checks the dedicated service SID with read-only
`sc.exe showsid`, and writes `target/envbox-policy-driver/result.json`. It does
not install/create a service, register a driver, sign a catalog or load anything.
The prototype has no unload handler and is not eligible for loading. See the
[adapter contract](../../drivers/envbox-policy/ADAPTER.md) for incomplete gates.
