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
# build (see scripts/build.ps1)
cargo build -p envbox-cli -p envbox-probe

# add a US profile and run Probe under it
envbox profile add --name US --locale en-US --ui-language en-US --region US `
  --tz-windows "Pacific Standard Time" --tz-iana America/Los_Angeles
envbox run --profile <id> .\target\debug\envbox-probe.exe
```

Config lives under `%LOCALAPPDATA%\EnvBox\` (or `ENVBOX_CONFIG_ROOT`).

## Performance

Extra launch latency target: ideal &lt; 100 ms, acceptable &lt; 300 ms. Runtime has no polling or timers (event-driven hooks only).

## Scope notes

- Virtual timezone, real timeline (never hook `GetSystemTime` / QPC / TickCount)
- DNS View only — no packet redirection
- Registry Virtual View is whitelist-only — not a registry sandbox
- No anti-stealth / anti-detection goals
