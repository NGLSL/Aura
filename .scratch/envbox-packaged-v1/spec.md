# Status: ready-for-agent

## Problem Statement

EnvBox 已证明 Process-scoped 环境虚拟化对普通 Win32 可行：`CreateProcess(CREATE_SUSPENDED) → 注入 envbox-runtime.dll → Resume`，子进程经 Hook 继承同一 Profile。但这条链路把「启动」和「注入」绑死在 CreateProcess 上，遇到 WindowsApps / MSIX 就走不通：

1. **Package Identity 不是路径问题** — 直接跑 `WindowsApps\...\xxx.exe` 会丢 package identity / activation context；官方入口是 `IApplicationActivationManager::ActivateApplication(AUMID)`。
2. **Win32 与 Packaged 启动机制不同，产品语义应相同** — 用户只要「同一 Profile 下的 Environment View」，不应关心目标是 exe 还是 Store 包。
3. **Packaged root 只能激活后附加** — 存在 early-start race；子进程仍可 pre-execution 注入。差异必须被建模成隔离保证级别，而不是 bug 或隐藏 if 分支。
4. **Environment Block 不是通用配置通道** — `ActivateApplication` 没有 `lpEnvironment`；Runtime 若只认 `ENVBOX_*`，packaged 根进程拿不到 Profile。
5. **按类型分支的启动器不可扩展** — 未来还有 PackageDebug / Sandbox / WSL 等；若上层到处 `if windowsapps`，每加一种后端都要改业务。

长期目标是通用 Windows 应用环境层：**启动方式可不同，Runtime / Profile / Hook / IPC / 子进程传播必须统一**。现在控制面还小，正是把控制层从「Launcher + Injector」升级为「Environment Session Runtime」的时机；Locale/Timezone 等 Hook 层无需大改。

## Solution

把系统重心从 PID/Injector 上移到 **EnvironmentSession**，启动与注入拆开，用 Capability 决定附加策略：

```
                EnvironmentSession
                       │
          ┌────────────┴────────────┐
          │                         │
    Target Resolver           Profile Engine
          │                         │
          ▼                         │
   Activation Router                │
   │              │                 │
Win32 Backend  Packaged Backend     │
   │              │                 │
CreateProcess  ActivateApplication  │
 SUSPENDED          │               │
   │               PID              │
   └──────┬────────┘                │
          ▼                         │
     Attach Manager ◀───────────────┘
          │
   envbox-runtime.dll
          │
   Unified Runtime Context
   (Locale/Geo/TZ/Language/DNS/Registry)
          │
     Process Tree
```

用户可见效果不变：选 Application + Profile → Run → 目标进程树读到一致 Environment View；Host 不变；注入失败不静默降级。

V1 交付范围（控制面重构 + 第一个非 Win32 后端）：

1. **EnvironmentSession** 成为一次 Run 的核心（替代仅以 RuntimeInstance PID 列表为中心的控制语义）。
2. **ActivationBackend / RuntimeAttacher** 拆分：Win32 走 PreExecution，Packaged mediumIL 走 PostActivation。
3. **Capability Engine** 用 TargetCapabilities / 进程探测结果选策略，不写 `if WindowsApps`。
4. **IPC Bootstrap**：Runtime 按 PID 向 Host/Broker 取 RuntimeProfile；Environment Block 仅作 Win32 回退。
5. **Packaged App Backend V1**：AUMID 激活 → PID → Probe → 仅 mediumIL 且无阻断 mitigation 时注入。

明确不做见 Out of Scope；AppContainer 仍是 Unsupported（Fail Closed）。

## User Stories

1. As a user, I want to run any supported Windows application under an Environment Profile without caring whether it is a classic exe or a Store packaged app, so that Geo/Locale/Timezone feel the same everywhere.
2. As a user, I want packaged apps launched only via AUMID activation, so that package identity and activation context are preserved.
3. As a user, I want classic Win32 apps to keep today’s pre-execution injection behavior, so that existing profiles and tools do not regress.
4. As a developer, I want activation and runtime attachment to be separate steps, so that new launch mechanisms plug in without rewriting injection.
5. As a developer, I want a single Environment Session model that tracks root and child processes plus optional package identity, so that instances are not grouped by exe name or parent pid alone.
6. As a user, I want the Runtime to learn its Profile over a PID-scoped IPC handshake, so that packaged roots without an Environment Block still virtualize correctly.
7. As a user, I want Win32 runs to still work if IPC is unavailable via the existing environment variables, so that the platform is not a hard cutover.
8. As a developer, I want TargetCapabilities (can_suspend, can_inject_runtime, can_use_environment_block, can_assign_job, can_track_children) instead of path/type checks, so that backend selection is data-driven.
9. As a user, I want an explicit IsolationGuarantee on each session (FullPreExecution / PostActivation / Partial), so that I understand residual race risk on packaged roots.
10. As a developer, I want AttachStrategy to include PreExecution, PostActivation, and a reserved PackageDebug slot, so that stronger packaged attach can be added later without redesign.
11. As a user, I want AppContainer or Microsoft/Store-signed-only targets to fail closed with a clear reason, so that I never assume virtualization that did not happen.
12. As a user, I want mediumIL Packaged Win32 without blocking mitigations to load the Runtime and honor Profile values, so that store desktop apps can join the same Environment View.
13. As a developer, I want package discovery metadata (DisplayName, AUMID, PackageFullName, PackageFamilyName, RuntimeBehavior, TrustLevel) available to picker/CLI, so that users see honest capability before launch.
14. As a user, I want child processes of any supported root to be created suspended, attached, then resumed, so that only the packaged root has a post-activation race.
15. As a developer, I want one RuntimeInjector shared by all attach strategies, so that remote-load/Detours logic is not duplicated per backend.
16. As a user, I want instance stop to kill the session process set cleanly and leave no package/debug configuration changes, so that Host and Store apps stay unpolluted.
17. As a developer, I want belongs_to_session to combine ancestry and package family + activation window, so that packaged trees with brokers/hosts still track correctly.
18. As a user, I want Startup Fail Policy to apply when activation works but injection is unsupported, so that a live unvirtualized store app is never presented as an EnvBox success.
19. As a developer, I want GUI and CLI to both start Environment Sessions through one seam, so that Aura does not grow a second launcher.
20. As a user, I want Probe to report the same virtualized fields for a packaged root as for a Win32 root under the same Profile, so that acceptance is comparable.
21. As a developer, I want Hook modules (timezone, locale, language, geo, registry, dns, process) unchanged by the control-plane split, so that virtualization semantics stay stable.
22. As a user, I want Host system configuration to remain untouched after packaged and Win32 matrix runs, so that EnvBox stays Host-transparent.
23. As a developer, I want IPC messages to cover HELLO / GET_PROFILE / PROCESS_CREATED / PROCESS_EXITED / RUNTIME_READY / HOOK_ERROR, so that session lifecycle is observable without guessing.
24. As a user, I want a documented V1 limitation for packaged root early-start race, so that apps that cache geo/locale before attach are explained rather than mysterious.
25. As a developer, I want future backends (PackageDebug, Sandbox, WSL) to attach to Activation/Attach seams only, so that the core session model does not churn.
26. As a user, I want `envbox run` to remain the primary acceptance entry, so that automation does not depend on GUI.
27. As a developer, I want RuntimeInstance (or its successor EnvironmentSession record) to persist package family / AUMID / isolation guarantee when present, so that audit and stop paths can reason about packaged sessions.
28. As a user, I want stop/exit of a packaged session to leave the package’s normal lifecycle alone (no debug mode, no suspend-on-demand policy change), so that apps behave as the Store intended.

## Implementation Decisions

- Vocabulary first in the domain glossary: **EnvironmentSession**, **ActivationBackend**, **AttachStrategy**, **TargetCapabilities**, **IsolationGuarantee**, **Runtime IPC Bootstrap**, **Package Identity**, **early-start race**. Existing **RuntimeInstance** / **LaunchTarget** / **Startup Fail Policy** stay; Session is the control-plane aggregate (RuntimeInstance remains the persisted run record name in storage unless a later migration renames it).
- Control-plane pipeline (stable order): Target resolve → Capability probe → Backend selection → Activation → Attach → Bootstrap → Track. No `if windowsapps` outside capability/backend selection.
- Core type shapes (decision, from architecture design — keep as contracts, not demo code):
  ```rust
  trait ActivationBackend {
      fn activate(&self, target: &LaunchTarget) -> Result<ActivatedTarget>;
  }
  trait RuntimeAttacher {
      fn attach(&self, target: &ActivatedTarget, session: &EnvironmentSession)
          -> Result<AttachedRuntime>;
  }

  struct ActivatedTarget {
      pid: u32,
      architecture: Architecture,
      package_identity: Option<PackageIdentity>,
      state: ProcessState, // suspended | running
      capabilities: TargetCapabilities,
  }

  struct TargetCapabilities {
      can_suspend: bool,
      can_inject_runtime: bool,
      can_create_environment_block: bool,
      can_assign_job: bool,
      can_track_children: bool,
  }

  enum AttachStrategy { PreExecution, PostActivation, PackageDebug /* reserved */ }

  enum IsolationGuarantee { FullPreExecution, PostActivation, Partial }
  ```
- `LaunchTarget` becomes an extensible enum: `Win32` / `Command` / `Packaged { aumid, package_family_name, package_full_name }`, with room for `Sandboxed` / `Wsl` / `Custom`. Serde must keep loading existing `executable` / `command` documents.
- Win32 backend: CreateProcess suspended → attach (inject) → handshake → resume. AttachMode conceptually PreExecution; `caller_requested_suspended` still means do not resume. IsolationGuarantee = FullPreExecution.
- Packaged backend: `ActivateApplication(AUMID)` → PID → capability inspect → attach if supported. IsolationGuarantee = PostActivation. Never launch `WindowsApps\...\exe` as the packaged root.
- Capability Engine rules (product policy, not heuristic UI only): AppContainer → Unsupported; MicrosoftSignedOnly / StoreSignedOnly → Unsupported; cannot open/query process → Unsupported; mediumIL packaged without blocking mitigation → Supported. Process-level probes: TokenIsAppContainer, integrity, ProcessSignaturePolicy, ProcessDynamicCodePolicy, ProcessImageLoadPolicy. **No bypass.**
- Attach selection from capabilities: `can_suspend` → PreExecution; else `can_inject_runtime` → PostActivation; else Unsupported / other backend.
- RuntimeInjector is one seam: inject runtime image into a PID regardless of how the process was created. Detours/remote-load implementation stays in the launcher/attach layer; Runtime C++ hooks are untouched.
- Runtime configuration channel: PID → Broker/Host Named Pipe (`\\.\pipe\envbox-runtime` or session-scoped name). Messages: HELLO, GET_PROFILE, PROCESS_CREATED, PROCESS_EXITED, RUNTIME_READY, HOOK_ERROR. Profile payload may later move to shared memory (`Local\EnvBox\Session\<UUID>`, runtime read-only); V1 may send profile over the pipe. Environment variables remain a Win32 fallback only.
- V1 ships the session manager **in-process** (CLI/GUI → session API). A separate `envbox-broker` process is the target architecture for GUI-outliving sessions and multi-client IPC, but is **not required** to accept V1; keep the protocol broker-ready.
- Child propagation is unified after root attach: CreateProcess* hooks force suspend → register child with session → attach → resume. No WindowsApps-vs-Win32 child code paths.
- Session membership: root descendants OR (package family match AND creation time within activation window). Never exe-name-only matching.
- Persistence: session/run record gains optional `package_family_name`, `aumid`, `isolation_guarantee`, `attach_strategy`. Job Object remains lifecycle-only (best-effort; packaged may have partial job support via capabilities).
- Fail Open stays for non-critical hooks; Fail Closed stays for cannot-activate / cannot-attach / unsupported capability.
- GUI/CLI call the same session start/stop API. UI work remains out of this spec.

## Testing Decisions

Good tests assert **external behavior** at the highest existing seams, not COM/Detours internals.

- **Primary seam (one product path):** `envbox run` (and library launch/session start used by CLI/GUI) plus **Probe** output for a Profile. Prior art: V0.1 acceptance matrix, `cli_run.rs`, `cli_boundary.rs`, probe tests.
- **Secondary seam (capability policy only):** pure rules table for Capability Engine (AppContainer / signed-only / mediumIL / cannot-open). Unit-testable without a store app.
- **IPC seam:** protocol round-trip (hello → profile payload) with a fake broker; Win32 env-fallback regression unchanged.
- Packaged acceptance (real mediumIL WindowsApps app on the machine):
  1. does not CreateProcess a WindowsApps exe as root
  2. AUMID activate returns PID
  3. probe = mediumIL / supported
  4. runtime loads (handshake success preferred over env marker)
  5. Probe geo/locale/timezone show Profile values
  6. child of packaged root still pre-start injected
  7. Host snapshot unchanged
  8. session stop leaves package config/lifecycle unpolluted
- Unsupported packaged: AppContainer or signature-blocked → session fails closed with reason; no silent unvirtualized process presented as success.
- Do not assert Detours helper ordinals, pipe byte layouts beyond the public message names, or hook call counts as product contracts.

## Out of Scope

- AppContainer / UWP / protected-process virtualization
- PSF unpack/re-sign or third-party package rewrite
- `IPackageDebugSettings` as default launch path (PackageDebug attach strategy stays reserved/experimental)
- Eliminating packaged root early-start race
- Changing Win32 pre-execution injection contract
- GUI redesign (UI frozen for this feature)
- Separate `envbox-broker.exe` process split (protocol-ready only in V1)
- WFP, DNS driver, sandbox/VM/WSL backends (future ActivationBackend only)
- Hook coverage expansion (audit-driven hooks stay a separate track)
- Hot-reload Profile into a live session

## Further Notes

**Re-analysis of the two plans together**

The first plan (Packaged App Backend V1) is the correct **vertical slice**. The second plan (Environment Session + multi-backend) is the correct **control-plane shape**. They are not alternatives:

- Without Session/Activate/Attach split, Packaged support becomes a permanent `if` in `launch()`.
- Without Packaged V1 as the first second backend, Session architecture is speculative.

Recommended sequencing (still one spec, tickets ordered):

1. Introduce Session + Activation/Attach traits **with Win32 still passing acceptance** (behavior lock).
2. Capability Engine + Packaged activation + probe policy.
3. Packaged attach + IPC bootstrap (env fallback preserved for Win32).
4. Unify child tracking and session membership (ancestry + package window).
5. Only then consider broker process / PackageDebug.

Hooks remain a unified Runtime Context — the product claim stays: different Windows Application Models, one Environment Session Runtime.

DLL injection is demoted to AttachStrategy #1, not the product identity.

**Tier model (docs, not code):** Tier 1 Native Win32 full pre-exec; Tier 2 Packaged Win32 mediumIL post-activation; Tier 3 AppContainer/protected unsupported or future strict backend. Do not promise one injection technique covers all Windows processes.

**Seams check:** acceptance stays at `envbox run` + Probe (existing). New logic concentrates behind session/activation/attach. If only one new test surface is allowed, prefer the capability policy unit seam; packaged end-to-end remains machine-conditional acceptance like existing notepad/Mimo tests.
