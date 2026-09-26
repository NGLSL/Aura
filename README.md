# EnvBox

Run Windows applications with isolated locale, region, timezone and network profiles — without a VM.

Windows process-level environment virtualization launcher. Target apps run on the host (full access to filesystem, GPU, network, user profile, Git/SSH/IDE) but see **Environment Profile** values for locale, region, UI language, timezone, DNS view, environment variables, and whitelisted internationalization registry reads.

## Security posture

**EnvBox does not provide a security boundary; launched apps keep the current user’s filesystem and permissions.**

EnvBox is not a sandbox. It does not isolate files, registry writes, network, or privileges. Do not use it as a security control.

### Elevation / integrity

Runtime injection is process-scoped and runs at EnvBox’s integrity level. A target that requires a **higher** integrity level (for example `requireAdministrator`) cannot be injected. Startup Fail Policy: EnvBox **refuses to start** the target rather than silently running it without virtualization. The error names `integrity/elevation` and states that the target must run at the **same integrity level as EnvBox or lower** — start EnvBox elevated when the target requires elevation. This is a compatibility limit, not a security control.

## Principles

1. **Process-scoped** — changes apply only to one RuntimeInstance’s process tree
2. **Host-transparent** — never modify Windows global configuration
3. **Environment-consistent** — one Profile keeps Locale / Region / Language / Timezone / DNS / Environment aligned

## Components

| Path | Role |
|------|------|
| `crates/envbox-app` | Iced GUI |
| `crates/envbox-core` | Domain model |
| `crates/envbox-storage` | TOML persistence + validation |
| `crates/envbox-launcher` | Resolve, env block, Job, injection |
| `crates/envbox-cli` | `envbox run` / `profile` / `app` |
| `runtime/` | Detours Runtime DLL |
| `tools/envbox-probe` | Acceptance probe |

## Quick start

```powershell
# Rust development build
cargo build

# Complete installer build (Rust workspace + x64/x86 Runtime + NSIS)
.\scripts\build-installer.ps1

# add a US profile and run Probe under it
envbox profile add --name US --locale en-US --ui-language en-US --region US `
  --tz-windows "Pacific Standard Time" --tz-iana America/Los_Angeles
envbox run --profile <id> .\target\debug\envbox-probe.exe
```

Config lives under `%LOCALAPPDATA%\com.aura.envbox\` (or `ENVBOX_CONFIG_ROOT`).

## Windows installer

`scripts/build-installer.ps1` builds the Rust workspace, both C++ Runtime DLLs, and the NSIS installer. The version shown in Aura and the version registered by the installer both come from `workspace.package.version` in `Cargo.toml`. When bumping it, update and commit `Cargo.lock` too; an explicit `-Version` value is accepted only when it matches the Cargo version.

For a test build in the private GitHub repository, run **Actions → Build Windows installer → Run workflow**. Download the `aura-windows-installer` artifact from the completed run; it contains `aura-setup.exe` and its SHA-256 checksum.

For a release, update `Cargo.toml` and `Cargo.lock`, add `docs/releases/v<version>.md`, then push `main` and wait for `ci.yml` to pass on that exact commit. Create and push an annotated `v<version>` tag on the same commit. `release.yml` checks the tag, version, `main` commit and CI result, builds the complete Windows installer, and publishes a GitHub Release with the installer and checksum. Both workflows build pinned Microsoft Detours for x64 and x86 on a Windows 2022 runner.

## Instance lifecycle

Closing Aura, or replacing Aura during an installer upgrade, leaves applications already launched by Aura running. Those detached instances keep the immutable Profile they received at launch, including for later child processes. The **Stop** action still explicitly terminates the tracked process tree. After Aura exits, its in-memory tracking is gone, so a reopened Aura cannot stop or inspect the detached instance.

## Performance

Extra launch latency target: ideal &lt; 100 ms, acceptable &lt; 300 ms. Runtime has no polling or timers (event-driven hooks only).

## Scope notes

- Virtual timezone, real timeline (never hook `GetSystemTime` / QPC / TickCount)
- DNS View only — no packet redirection
- Registry Virtual View is whitelist-only — not a registry sandbox
- No anti-stealth / anti-detection goals
