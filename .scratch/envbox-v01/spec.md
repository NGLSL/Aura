# Status: ready-for-agent

## Problem Statement

在中文（或其他非目标）Windows 宿主上运行 Claude Code / Agent / 编译器等工具时，进程读取到的 Locale、Region、Timezone、UI Language、DNS 配置往往与目标环境不一致，导致输出语言、时间戳、地区相关格式、DNS 解析视图出现偏差。用户需要同一套代码在不同“环境人格”下可重复运行，又不想改动宿主全局配置，也不想引入虚拟机、文件系统隔离或完整沙箱带来的性能与权限代价。现有方案要么修改系统设置（影响整个用户会话），要么过于隐身/对抗向，都不是“可重复、内部一致的环境虚拟化”。

## Solution

EnvBox 是 Windows 原生应用启动器：用户为 Application 绑定 Environment Profile，通过 EnvBox 启动后，目标进程树在仍可完整访问宿主文件系统、GPU、网络、用户目录、Git/SSH/IDE 的前提下，读取部分系统环境信息时得到 Profile 指定的值（例如 Host `zh-CN`/`CN`/`China Standard Time`，Profile `en-US`/`US`/`Pacific Standard Time`）。隔离单位是 Process Tree Instance，Host 配置完全不被修改。V0.1 提供 Application/Profile 管理、CLI/启动注入、Locale/Timezone/Geo/UI Language/DNS View/白名单 Registry 虚拟化、子进程继承，以及 Probe 验收基准；GUI 最后做。

## User Stories

1. As a developer running agents on a Chinese Windows host, I want to launch a tool under a US Environment Profile, so that its locale, region, and timezone read as US without changing my system settings.
2. As a user, I want to create an Application entry with a name, so that I can identify and manage the tool I launch through EnvBox.
3. As a user, I want to configure an Application with an Executable path, so that I can launch a concrete `.exe` through EnvBox.
4. As a user, I want to configure an Application with a Command (for example `claude`), so that PATH-resolved CLIs and wrappers work without hard-coding install paths.
5. As a user, I want to set Arguments and Working Directory on an Application, so that the target starts in the correct project context.
6. As a user, I want to bind a default Environment Profile to an Application, so that every Run uses a consistent environment unless I override it.
7. As a user, I want to edit and delete an Application, so that my launch list stays accurate over time.
8. As a user, I want to create and name an Environment Profile, so that I can reuse one environment persona across many Applications.
9. As a user, I want to set Profile locale_name, ui_language, and region together, so that Locale APIs never disagree within the process tree.
10. As a user, I want to set both Windows timezone ID and IANA timezone ID on a Profile, so that conversion uses Windows DST rules and stays consistent for consumers that read either ID.
11. As a user, I want DNS Mode Host or VirtualView with explicit server addresses, so that tools enumerating DNS config see the intended view without transparent traffic redirection.
12. As a user, I want to define Profile environment variables (for example `LANG`, `LC_ALL`, `TZ`, proxy vars), so that POSIX-style tools inherit the same persona.
13. As a user, I want Profile Registry Virtual View limited to an internationalization whitelist, so that only locale/timezone-related registry reads are virtualized.
14. As a user, I want Profile validation before save, so that invalid locale, region, timezone, DNS, or environment names cannot be persisted.
15. As a user, I want each Run to create a distinct RuntimeInstance, so that I can run the same Application under multiple environments concurrently.
16. As a user, I want to see RuntimeInstance status (Starting / Running / Stopping / Exited / Failed) in the GUI, so that I know whether the process tree is still alive.
17. As a user, I want the Instance to stay Running while any child remains in the Job Object after Root Process exit, so that I do not mistake a live tree for a finished run.
18. As a user, I want one-click Stop of a RuntimeInstance, so that the entire process tree can be torn down together.
19. As a user, I want to see child process count for a running Instance, so that I understand what the launched tool spawned.
20. As a user, I want Environment Block built by the Launcher (clone host env, apply Profile overrides, add EnvBox internal IDs) and passed via CreateProcess, so that environment variables do not rely on fragile hooks.
21. As a user, I want `ENVBOX_INSTANCE_ID` and `ENVBOX_PROFILE_ID` visible inside the process tree, so that Runtime can load the correct Profile and debugging stays possible.
22. As a user, I want target processes created suspended and Runtime injected before application logic runs, so that early API reads already see the Profile.
23. As a user, I want timezone APIs (`GetTimeZoneInformation`, `GetDynamicTimeZoneInformation`, and related conversion APIs) to return Profile timezone, so that local time formatting matches the Profile.
24. As a user, I want absolute time APIs (`GetSystemTime`, `QueryPerformanceCounter`, `GetTickCount`, etc.) to remain real, so that the virtual timezone never fabricates the timeline.
25. As a user, I want Geo APIs (`GetUserDefaultGeoName`, `GetUserGeoID`) to return the Profile region, so that region-sensitive logic sees e.g. `US` instead of `CN`.
26. As a user, I want Locale APIs (`GetUserDefaultLocaleName`, `GetSystemDefaultLocaleName`, LCID and `GetLocaleInfo*`) to consistently map to the Profile locale, so that no API pair can return `en-US` and `zh-CN` at once.
27. As a user, I want UI Language APIs and preferred language lists to put the Profile language first, so that resource loaders pick the intended UI language.
28. As a user, I want `GetNetworkParams` / `GetAdaptersAddresses` to report Profile DNS servers under VirtualView, so that DNS configuration probes match the Profile without packet redirection.
29. As a user, I want whitelisted Registry reads under internationalization/timezone keys to return virtual values, so that registry-based readers stay consistent with API hooks.
30. As a user, I want all non-whitelisted Registry access to pass through to Windows, so that EnvBox is not a Registry Sandbox and compatibility stays high.
31. As a developer of multi-process tools, I want child processes created via `CreateProcessW/A` to inherit the Profile, so that `node`, `git`, `powershell`, `python` all see the same environment.
32. As a user, I want CreateProcess hooks to force `CREATE_SUSPENDED`, inject Runtime, then resume only if the caller did not already request suspension, so that caller-owned suspend semantics are preserved.
33. As a user, I want `caller_requested_suspended = true` cases to remain suspended after injection, so that debuggers and custom launchers keep control.
34. As a user, I want isolation scoped to the Process Tree Instance, so that a PowerShell window I open from the taskbar is never affected.
35. As a user, I want Job Object tracking with kill-on-close, so that lifecycle statistics and Stop are reliable even though Job Object is not a security boundary.
36. As a user, I want `.cmd` / `.bat` wrappers resolved through `%ComSpec% /d /s /c`, so that tools like `claude` that are batch wrappers still start correctly.
37. As a user, I want PATH resolution to try `.exe`, `.com`, `.cmd`, `.bat`, so that Command LaunchTargets behave like a shell.
38. As a user, I want envbox-probe to print GEO/LOCALE/LANGUAGE/TIMEZONE/DNS/ENV snapshots, so that Host vs Profile differences are objective.
39. As a user, I want `envbox-probe --spawn-child` to verify parent/child Profile inheritance, so that propagation is proven before real agents are trusted.
40. As a user, I want Host and US Profile probe outputs to differ only in virtualized fields, so that Host transparency is demonstrated by comparison.
41. As a user, I want `envbox run --profile <id> <command>` from CLI before GUI exists, so that Runtime can be validated early.
42. As a user, I want `envbox profile list` and `envbox app list`, so that configuration is inspectable without GUI.
43. As a user, I want Startup Fail Policy: missing Runtime DLL, missing/corrupt Profile, process creation or injection failure aborts with a clear reason, so that I never believe an app is virtualized when it is not.
44. As a user, I want Fail Open inside hooks (fallback to original Windows API) except for fatal Runtime init failure, so that a bad Profile mapping does not crash the target app.
45. As a user, I want Profile immutable per RuntimeInstance and changes effective on next launch, so that a running tree cannot see a half-applied config.
46. As a user, I want optional “Child processes inherit profile” on Application, so that I can disable propagation when I only want the root process virtualized.
47. As a user, I want Run With (temporary Profile selection) without changing the default Profile, so that one-off experiments do not rewrite configuration.
48. As a user, I want About/README to state that EnvBox is not a security boundary, so that filesystem and user permissions are not misrepresented.
49. As a user, I want target processes to keep normal access to `C:\`, `D:\`, Git repos, SSH keys, and project files, so that development workflows are uninterrupted.
50. As a user, I want Host timezone, language, region, and DNS to remain completely unmodified after any Run, so that the rest of the OS is unaffected.
51. As a user, I want GUI Application editor fields (Name, Launch type, path/command, Arguments, Working Directory, Default Profile, inherit flag) with Save/Run/Delete, so that day-to-day management is practical.
52. As a user, I want GUI Profile editor fields (Region, Locale, UI Language, Timezone from Windows enumeration, DNS, Environment Variables) with validation, so that profiles are consistent and legal.
53. As a user, I want Timezone dropdown populated from Windows timezone enumeration rather than a hand-maintained list, so that Windows IDs are always valid.
54. As a user, I want Runtime logging at INFO by default without recording secrets, file contents, or sensitive API payloads, so that debug trails are safe.
55. As a user, I want optional Runtime Debug Mode that logs hook hits, so that I can diagnose virtualization coverage without default noise.
56. As a developer, I want Runtime hooks split by domain module (timezone, locale, language, geo, registry, dns, process) rather than one giant hooks file, so that coverage can grow incrementally.
57. As a developer, I want P0 hooks first and Probe-driven expansion, so that we do not intercept dozens of APIs before the acceptance harness exists.
58. As a user launching high-integrity tools, I want a clear elevation mismatch message instead of a silent broken run, so that I can restart EnvBox at the required integrity level.
59. As a user, I want x64 target support first with both `envbox-runtime32.dll` and `envbox-runtime64.dll` artifacts prepared, so that architecture expansion does not require redesign.
60. As a user, I want Proxy environment variables in Profile to be best-effort, so that I understand public IP and account region are out of scope.

## Implementation Decisions

- Domain model separates **Application**, **Environment Profile**, **RuntimeInstance**, and **Process** / **Process Tree Instance**; they must not be collapsed into one concept. Vocabulary follows `docs/CONTEXT.md`.
- Modules: `envbox-app` (Iced GUI), `envbox-core` (domain), `envbox-storage` (TOML persistence + validation), `envbox-launcher` (command resolve, environment block, job, injection), `envbox-cli`, C++ `runtime` (Microsoft Detours), `tools/envbox-probe`.
- Persistence lives under `%LOCALAPPDATA%\EnvBox\` as `config.toml`, `applications.toml`, `profiles.toml`, and `logs\`.
- LaunchTarget is either Executable (path) or Command (string). Command resolution order: full EXE → PATH `.exe`/`.com`/`.cmd`/`.bat` → `.cmd`/`.bat` wrapped as `%ComSpec% /d /s /c "<command>"`. PowerShell scripts are not auto-recognized in V0.1.
- Profile contains LocaleProfile (locale_name, ui_language, region), TimezoneProfile (windows_id + iana_id), DnsProfile (Host | VirtualView + servers), environment map, and RegistryProfile whitelist. DST is never hand-computed; Windows timezone rules produce conversions.
- RuntimeInstance is created per Run and tracks application_id, profile_id, root_pid, process_ids, started_at, and InstanceStatus. Status remains Running while Job children survive Root Process exit.
- Launch sequence is fixed: resolve Application → resolve Profile → resolve LaunchTarget → build Environment Block → allocate RuntimeInstance ID → create Job Object → CreateProcess suspended → inject Runtime → assign Job → init Runtime Profile → resume main thread → Running. Injection failure rejects the start (Startup Fail Policy); no silent unvirtualized launch.
- Environment variables are applied primarily via a cloned, overridden Unicode Environment Block (`CREATE_UNICODE_ENVIRONMENT`) rather than hooks. Internal vars `ENVBOX_INSTANCE_ID` / `ENVBOX_PROFILE_ID` are not hidden in V0.1. Runtime loads Profile from `profiles.toml` using those IDs (Shared Memory / Detours Payload / Named Pipe are deferred).
- Timezone virtualization principle: **Virtual timezone, real timeline**. Hook timezone query and local conversion APIs; never hook or alter `GetSystemTime`, `GetSystemTimeAsFileTime`, `GetSystemTimePreciseAsFileTime`, `QueryPerformanceCounter`, `GetTickCount`. Do not call `SetDynamicTimeZoneInformation`.
- Geo primary hook is `GetUserDefaultGeoName` (ISO 3166-1 alpha-2); `GetUserGeoID` is P1. Locale and UI Language hooks must map consistently to the same Profile locale/LCID/language list (Profile language first in preferred lists).
- DNS V0.1 is **DNS View** only (`GetNetworkParams`, `GetAdaptersAddresses`). No raw UDP/TCP 53, DoH, WFP, or transparent proxy.
- Registry Virtual View is whitelist-only (`HKCU\Control Panel\International`, `HKLM\SYSTEM\CurrentControlSet\Control\TimeZoneInformation`, plus Probe-discovered related keys) via `RegOpenKeyExW` / `RegQueryValueExW` / `RegGetValueW`; everything else passes through.
- Child propagation hooks `CreateProcessW/A` first; `CreateProcessAsUser*` / `WithTokenW` / `WithLogonW` are P2. Hooks force `CREATE_SUSPENDED`, inject Runtime, ensure Profile ID inheritance, then ResumeThread only when `caller_requested_suspended` is false.
- Job Object per RuntimeInstance uses `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` for lifecycle/stop/stats only — not a security boundary. Profile inheritance still depends on child injection.
- DllMain path: `PROCESS_ATTACH` → `DetourIsHelperProcess` → `DetourRestoreAfterWith` → Load Profile → Install Hooks. Produce both `envbox-runtime32.dll` and `envbox-runtime64.dll`; V0.1 prioritizes x64 targets. Launch uses `DetourCreateProcessWithDllEx`.
- Runtime Profile is parsed once at DLL init into an immutable `std::shared_ptr<const RuntimeProfile>`; hooks are read-only and lock-light. No hot reload in V0.1.
- Fail Open: most hook errors fall back to the original Windows API. Only complete Runtime init failure during startup is fatal.
- Logging: Rust `tracing`; Runtime simple file log at INFO. No file contents, shell command bodies, tokens, or sensitive API payloads. Hook-hit logs only in Debug Mode.
- Hook modules stay split (`timezone`, `locale`, `language`, `geo`, `registry`, `dns`, `process`); forbid a single multi-thousand-line hooks file. Formatting hooks (`GetDateFormatEx` etc.) are reserved but V0.1.1.
- Hook priority: P0 = timezone query, `GetUserDefaultGeoName`, user/system default locale names, user/system default UI languages, `GetUserPreferredUILanguages`, DNS view APIs, `CreateProcessW/A`. P1 = timezone-for-year, GeoID, LCIDs, remaining preferred-UI-language APIs, whitelist Registry. P2 = formatting APIs and advanced CreateProcess family.
- GUI is Phase 9 and only calls core/launcher; injection logic never lives in UI. Application editor and Profile editor fields follow the product spec; Timezone list is Windows-enumerated.
- Security posture text in About/README: EnvBox does not provide a security boundary; launched apps keep the current user’s filesystem and permissions.
- Performance targets: extra launch latency ideal &lt; 100 ms, acceptable &lt; 300 ms; idle GUI CPU ≈ 0%; Runtime has no polling or timers.

## Testing Decisions

- Good tests assert external behavior at agreed seams only (Probe snapshots, CLI stdout/exit codes, public library contracts). Do not assert Detours internals, hook call counts, or private Rust modules. Expected values are independent literals from the Profile fixture or Host baseline, never recomputed the same way as production code.
- Seams are staged with implementation (confirmed order): **Probe → CLI → Injection → 4 core APIs → Child propagation → GUI last**. Prefer the highest seam that can see the behavior; do not add seams beyond this ladder.
- **Probe seam (highest, acceptance)**: `envbox-probe` / `envbox run --profile … envbox-probe` process I/O. Compare Host snapshot vs Profile snapshot; `--spawn-child` must match parent virtualized fields; non-virtualized fields and Host system config must be unchanged. This is the V0.1 Done Definition harness.
- **CLI seam**: `envbox run` / `profile list` / `app list` process I/O for command resolution, environment merge, working directory, argument quoting, cmd wrapper, and Startup Fail Policy messages.
- **Injection seam**: suspended create + Detours load observable as successful Probe run with Runtime loaded (initially “EnvBox Runtime Loaded”), without modifying API results yet.
- **Four core API seam**: Probe proves `GetDynamicTimeZoneInformation`, `GetUserDefaultGeoName`, `GetUserDefaultLocaleName`, `GetUserDefaultUILanguage` before wider locale/language/dns/registry coverage.
- **Child propagation seam**: Probe parent→child, then `cmd`→`node` (and `git` / `powershell` / `python` in the matrix). Same `ENVBOX_INSTANCE_ID` / Profile view across the tree; unrelated processes unaffected.
- **GUI last**: thin smoke over core/launcher (edit/save/run/stop status), not a substitute for Probe acceptance.
- Unit-level coverage (core/storage/launcher pure contracts) only where Probe cannot isolate cheaply: profile serialization, validation, application serialization, command resolution, environment merge. Prior art: Kite-style Rust regression tests around public modules; no code exists in this repo yet — start with `cargo test` in `envbox-core` / `envbox-storage` / `envbox-launcher`.
- Runtime integration cannot be fully unit-tested; `envbox-probe` is the integration authority. Test matrix at minimum: `envbox-probe`, `cmd`, `powershell`, `git`, `node`, `python`, `notepad`, plus a real Node CLI agent scenario.
- Host transparency is part of acceptance: after runs, Host timezone/language/region/DNS must match the pre-run Host snapshot.

## Out of Scope

- Virtual machines, Windows Sandbox, Hyper-V backends, filesystem isolation, full Registry sandbox
- CPU / GPU / BIOS / disk serial virtualization, MAC spoofing, user SID or Windows username virtualization
- Anti-debug, hidden DLL injection, hiding EnvBox, bypassing integrity checks or security software
- Browser fingerprint spoofing, public IP change, account/region changes on remote services
- Transparent raw UDP/TCP DNS interception, DoH interception, WFP drivers, packet redirection, transparent network proxies (later Network Mode)
- x86 as V0.1 primary (artifacts reserved; x86 Runtime is V0.2)
- Formatting API virtualization (`GetDateFormatEx` family) in V0.1 initial (V0.1.1+)
- Hot-reloading Profile for a live RuntimeInstance
- Strict Mode / sandboxed backends (V0.4)
- Stealth or anti-detection goals of any kind

## Further Notes

- Core principles to preserve in every change: Process-scoped, Host-transparent, Environment-consistent. The product goal is deterministic environment virtualization, not stealth.
- Phase order is intentional: Scaffold → Core → Launcher CLI → Injection (no hooks) → Minimal Profile (4 APIs) → Child Propagation → Complete Locale Layer → DNS View → Registry View → GUI. First agent batch stops after scaffold/core/cli/injection/runtime-load/commit, and must not start GUI, DNS, or Registry hooks.
- Acceptance commands before real Hook development: `cargo build`; `envbox-probe.exe`; `envbox profile list`; `envbox app list`; `envbox run --profile us .\envbox-probe.exe` with Probe success, Runtime loaded, and zero Host config change.
- Not a sandbox: document the security boundary disclaimer in About/README before any release claim.
