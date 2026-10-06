# EnvBox

Run Windows applications with consistent Profile environment information — without a VM.

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

Install [rustup](https://rustup.rs/) with the Windows MSVC toolchain. This repository pins Rust 1.99.0 in `rust-toolchain.toml`; Cargo selects it automatically. CI and installer workflows use the same version.

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

For a test build, run **Actions → Build Windows installer → Run workflow**. Download the `aura-windows-installer` artifact from the completed run; it contains `aura-setup.exe` and its SHA-256 checksum.

For a release, update `Cargo.toml` and `Cargo.lock`, add `docs/releases/v<version>.md`, then push `main` and wait for `ci.yml` to pass on that exact commit. Create and push an annotated `v<version>` tag on the same commit. `release.yml` checks the tag, version, `main` commit and CI result, builds the complete Windows installer, and publishes a GitHub Release with the installer and checksum. Both workflows build pinned Microsoft Detours for x64 and x86 on a Windows 2022 runner.

### Updates

Open **Settings → About Aura and updates** to check the latest stable release, open its release page, or visit the [GitHub repository](https://github.com/NGLSL/Aura). Update checks are manual and contact the GitHub API.

When a newer version is available, choose **Download and install**. Aura downloads the official release installer and its SHA-256 checksum, verifies the download, and opens the Windows installer. Windows may ask for administrator approval. Aura exits only after the installer starts successfully; finish the installation wizard and reopen Aura afterwards. Applications already launched through Aura keep running, and your local configuration is retained. If checking, downloading, verification, or installer startup fails, the error stays visible and you can retry or use the release page to install manually.

## Instance lifecycle

An **environment container** is a persistent workspace that selects a Profile and manages runs using immutable configuration snapshots. It changes the supported environment information that applications read; applications retain host resources and permissions. Container runs use the user-level Supervisor for background ownership, reconnect and Stop. No driver or LocalSystem service is required for this environment-information workflow.

Closing Aura leaves launched applications running. Container runs can be managed after reconnecting to their Supervisor; recovery must reconfirm process generations and Runtime identity, and reports TrackingLost when ownership cannot be established. Legacy direct runs retain their detached-instance behavior: reopening Aura does not automatically take ownership of their process trees.

The container view and CLI show observed Profile matching, configuration completeness and installed Hook groups. Hook installation is evidence of attachment, not proof that every API or application behaves as configured. Unobserved legacy records remain unknown. Historical storage policy metadata is retained for configuration compatibility, but is not enforced or exposed as an editable container feature.

```powershell
envbox container create --name US --profile <profile-uuid>
envbox container list
```

## Performance

Extra launch latency target: ideal &lt; 100 ms, acceptable &lt; 300 ms. Runtime has no polling or timers (event-driven hooks only).

## Scope notes

- Virtual timezone, real timeline (never hook `GetSystemTime` / QPC / TickCount)
- DNS configuration view and process-scoped Windows resolver routing — no packet redirection
- Registry Virtual View is whitelist-only — not a registry sandbox
- No anti-stealth / anti-detection goals

### DNS routing

With `VirtualView` and working DNS hooks, supported `DnsQuery_A/W/UTF8` and version-1 `DnsQueryEx` queries send every resource-record type to the Profile DNS servers, including HTTPS (65), SVCB (64), TXT, PTR, SRV and unknown types. Responses are decoded by Windows' DNS message parser; record formats depend on the installed Windows version. Profile timeouts, DNS errors and unsupported query inputs return an error instead of retrying through host DNS. Query names currently require ASCII (including pre-encoded IDN punycode); Unicode names return an error. `Host` mode keeps Windows resolution unchanged.

Profile DNS uses an explicit ordered list of UDP, TCP, DoT or DoH upstreams and requires `strict = true`. UDP truncation retries over TCP to the same endpoint. DoT uses a configured IP and TLS server identity; DoH uses configured bootstrap IPs and the HTTPS server identity. TLS trust, protocol or upstream failures return a resolution error or try the next explicitly configured upstream. A plaintext fallback occurs only when you configure a later UDP/TCP upstream. IPv4 upstream connections are the current delivery scope; IPv6 transport qualification is deferred, while AAAA records remain supported. See [.scratch/dns-transports/spec.md](.scratch/dns-transports/spec.md).

Supported Profile lookups through address-resolution hooks (`getaddrinfo` / `GetAddrInfo*`) return a resolution error when Profile DNS fails. Unsupported asynchronous resolver inputs are rejected without calling Windows resolution; `DnsQueryRaw`, when present and hooked, is explicitly rejected in strict Profile mode. Required DNS hook failures reject Runtime initialization. Local-machine passthrough, uninjected processes and application-owned UDP/TCP DNS, DoH, DoT or DoQ remain outside this resolver guarantee. Aura does not provide a network security boundary or change the host DNS configuration.

### Profile identity read views

Profiles optionally configure `identity.computer_name`, `user_name`, `mac_address`, and `machine_guid`. Empty fields retain the Host view. The GUI exposes these in Profile advanced settings; the CLI provides `profile identity show UUID`, `set UUID --computer-name NAME --user-name NAME --mac-address MAC --machine-guid UUID`, `set UUID --clear FIELD`, and `reset UUID`.

The current read APIs are `GetComputerNameA/W`, `GetComputerNameExA/W`, Winsock `gethostname` / `GetHostNameW`, `GetUserNameA/W`, `GetAdaptersAddresses`, `GetAdaptersInfo`, `GetIfEntry`, `GetIfTable`, `GetIfEntry2`, `GetIfTable2`, and `RegQueryValueExA/W` / `RegGetValueA/W` for `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`. Computer DNS domain views are empty, and the FQDN view is the configured single label. MAC applies to six-byte physical addresses returned by the listed APIs, including permanent addresses in MIB rows. With an explicit computer name, that virtual label cannot bypass strict Profile DNS as a local-machine lookup.

These views do not rename the host, change accounts, tokens, SIDs, permissions, adapter settings, registry data, or public IP. `GetUserNameEx`, WMI, Native Registry APIs, device IOCTLs and application-owned identity caches remain outside this API coverage. CPU/GPU/disk identity support is deferred. New immutable snapshots use Profile schema 3; legacy schema 1/2 snapshots retain their original digests. An identity Profile requires a Runtime that explicitly advertises and installs the necessary hooks.

## Community

Thanks to the [LINUX DO](https://linux.do/) community for supporting open-source projects.

## License

Aura / EnvBox is licensed under the [Apache License 2.0](LICENSE).

Microsoft Detours retains its MIT license; see [third-party notices](THIRD_PARTY_NOTICES.txt). Other dependencies retain their respective licenses.
