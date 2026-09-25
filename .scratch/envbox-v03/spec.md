# Status: ready-for-agent

## Problem Statement

EnvBox 已经证明 Process-scoped 环境虚拟化在普通 Win32 上成立：`CreateProcess(CREATE_SUSPENDED) → 注入 envbox-runtime.dll → Resume`，Time / Geo / Locale / Language / DNS / Registry Hook 与子进程传播都可用，Audit 也在。

但当前产品形态仍是一台 **Win32 DLL 注入器**，控制面把「启动」和「注入」绑死在 CreateProcess 链路上。用户面对的现实问题是：

1. **WindowsApps / MSIX 走不进同一条链路** — 官方入口是 AUMID 激活，不是 WindowsApps 路径下的 exe；`ActivateApplication` 没有 Environment Block，现有 `ENVBOX_*` 配置通道在 packaged root 上失效。
2. **配置被解析了两遍** — Rust 已经用 serde 解析 Profile，C++ Runtime 又用自制 TOML 解析读同一份 `profiles.toml`。规则漂移、字段遗漏、验证不一致都会变成运行时的神秘差异。
3. **实例模型默认「一定会有一个 root process」** — 真实应用可能是 AUMID 激活出的多进程树、Broker/COM 进程、package family 内的伴生进程；只记 root pid + 子 pid 会丢会话边界。
4. **再晚改动更大** — GUI、更多 Hook、更多应用类型一旦叠上去，`if windowsapps` / `if packaged` 会渗进业务层。现在正是把「DLL 注入工具」升级为「Windows 应用环境运行时」的时间点。

用户要的不是 Docker 式隔离，而是：**让一个进程树稳定地看到一套指定 Environment View**，且启动方式可以不同（Win32 / Command / Packaged），Runtime 与 Profile 语义必须统一。

## Solution

在已验证的 Runtime Hook 层之上演进控制面，把产品升级为 **Environment Session Engine**：

```
                    Application
                         │
                EnvironmentSession
                         │
                 Activation Router
                  /              \
         Win32 Backend        Package Backend
              │                    │
       CreateProcess          AUMID Launch
        (SUSPENDED)                │
              │                    │
              └─────────┬──────────┘
                        │
                 Runtime Attach
                        │
              envbox-runtime.dll
                        │
           Environment Virtualization
```

对用户可见的产品承诺不变：选 Application + Profile → Run → 进程树读到一致 Environment View；Host 不改；注入失败不静默降级。

V0.3 交付重心（在现有架构上演进，不推倒 Runtime）：

1. **EnvironmentSession** 成为一次 Run 的控制面聚合（目标、启动方式、Attach 策略、root/子进程、Package Identity、状态）。
2. **Activation / Attach 分离** — ActivationBackend 负责「得到一个活着的 PID」；RuntimeAttach 负责「把 envbox-runtime 装上去」。Win32 保持 PreExecution，Packaged 走 PostActivation。
3. **Capability Probe 前置** — 先测 Process Type / Architecture / Integrity / AppContainer / Mitigation，再决定 `can_inject`；AppContainer / Protected Process 直接 Fail Closed。
4. **Broker 成为配置与会话中枢** — `envbox-broker` 维护 Session Registry（PID → Profile），Runtime 经 Named Pipe 握手取 RuntimeProfile；`ENVBOX_*` 仅作回退。
5. **删除 C++ TOML 解析** — Rust serde → RuntimeProfile DTO → IPC → C++；C++ 只持有结构化字段，不读文件、不解析 TOML、不校验配置。
6. **子进程与会话归属升级** — Parent Runtime 通知 Broker（parent/child PID），Broker 把 child 归入 Session；后续 WindowsApps / Broker Process / COM Process 都能进同一 Session。
7. **生命周期跟踪双轨** — Win32 继续 Job Object；Packaged 用 PID + Package Identity；上层统一为 Process Tracker。

## User Stories

1. As a user, I want to run a classic Win32 exe under an Environment Profile exactly as today, so that existing Probe acceptance and tools do not regress.
2. As a user, I want to run a Store / WindowsApps application under the same Environment Profile semantics, so that Geo/Locale/Timezone feel identical across app models.
3. As a user, I want packaged apps launched only via AUMID activation, so that package identity and activation context are preserved.
4. As a user, I want Runtime profile delivery to work when the process has no Environment Block, so that packaged roots virtualize correctly.
5. As a user, I want Win32 runs to keep working if Broker is down via the existing ENVBOX_* channel, so that upgrades are not a hard cutover.
6. As a developer, I want activation and runtime attachment to be separate seams, so that new launch backends plug in without rewriting injection.
7. As a developer, I want one EnvironmentSession that records activation kind, attach strategy, root processes, child processes, and optional package identity, so that sessions are not grouped by exe name or a single root pid.
8. As a developer, I want a shared ActivationBackend contract (Win32 / Command / Packaged), so that the session pipeline never branches on WindowsApps path strings.
9. As a developer, I want a Capability Probe before any inject attempt, so that unsupported targets fail closed with a clear reason instead of half-starting.
10. As a user, I want AppContainer and protected processes to be rejected up front, so that I never see an unvirtualized process presented as an EnvBox success.
11. As a user, I want mediumIL Packaged Win32 without blocking mitigations to load the Runtime and honor Profile values, so that desktop Store apps join the same Environment View.
12. As a developer, I want a Broker process that owns Session Registry and PID → Profile mapping, so that Runtime bootstrap does not depend on Environment Variables or file parsing.
13. As a developer, I want Runtime to handshake with Broker by PID (HELLO → Profile), so that any attached process can resolve its RuntimeProfile without reading disk config.
14. As a developer, I want C++ Runtime to stop parsing TOML and only consume a RuntimeProfile DTO, so that Profile validation lives in one place (Rust serde).
15. As a user, I want child processes to inherit the same Profile through session registration rather than only ENVBOX env inheritance, so that multi-process and packaged trees stay consistent.
16. As a developer, I want Parent Runtime to report parent/child PID to Broker on process create, so that Broker can attach children to the correct EnvironmentSession.
17. As a user, I want session stop to terminate the tracked process set cleanly and leave Host / package lifecycle untouched, so that Store apps and system config stay unpolluted.
18. As a developer, I want Process Tracker to combine Job Object tracking (Win32) and Package/PID tracking (Packaged), so that one stop/audit path works for both.
19. As a user, I want an explicit IsolationGuarantee on each session (FullPreExecution / PostActivation / Partial), so that packaged-root early-start race is documented product behavior, not a mystery bug.
20. As a developer, I want AttachStrategy to include PreExecution, PostActivation, and a reserved PackageDebug slot, so that stronger packaged attach can be added later without redesign.
21. As a user, I want Startup Fail Policy to apply when activation works but injection is unsupported or Runtime profile cannot be resolved, so that a live unvirtualized app is never a silent success.
22. As a developer, I want GUI and CLI to start Environment Sessions through one session API, so that Aura does not grow a second launcher.
23. As a user, I want Probe to report the same virtualized fields under the same Profile regardless of Win32 vs Packaged vs Broker-attached root, so that acceptance stays comparable.
24. As a user, I want Host system configuration to remain unchanged after matrix runs, so that EnvBox stays Host-transparent.
25. As a developer, I want IPC messages (HELLO / GET_PROFILE / PROFILE / PROCESS_CREATED / PROCESS_EXITED / RUNTIME_READY / HOOK_ERROR) as a stable contract, so that Broker and Runtime can evolve independently.
26. As a user, I want Audit to keep working across the session model change, so that observational coverage does not regress while control plane moves.
27. As a developer, I want package discovery metadata (DisplayName, AUMID, PackageFamilyName, PackageFullName, RuntimeBehavior, TrustLevel) available to picker/CLI, so that users see honest capability before launch.
28. As a user, I want TargetCapabilities (can_suspend / can_inject_runtime / can_create_environment_block / can_assign_job / can_track_children) to drive strategy selection, so that support decisions are data-driven and testable.
29. As a developer, I want Runtime hooks (time/geo/locale/language/dns/registry/process) unchanged by the control-plane upgrade, so that virtualization semantics stay stable while the platform grows.
30. As a user, I want a documented V1/V0.3 limitation for packaged root early-start race, so that apps that cache geo/locale before attach are explained.
31. As a developer, I want future backends (PackageDebug, Sandbox, WSL) to attach only at Activation/Attach seams, so that the session core does not churn.
32. As a user, I want `envbox run` to remain the primary acceptance entry, so that automation does not depend on GUI.
33. As a developer, I want persisted run records to keep package family / AUMID / isolation guarantee / attach strategy when present, so that audit and stop paths can reason about packaged sessions.
34. As a user, I want stop/exit of a packaged session to leave the package’s normal lifecycle alone (no debug mode, no suspend-on-demand policy change), so that apps behave as the Store intended.
35. As a developer, I want RuntimeProfile DTO field mapping to be the single contract between Rust and C++, so that adding a virtualized field has one wire definition and one consumer struct.
36. As a user, I want Broker unavailable + ENVBOX available to still virtualize Win32, and Broker unavailable + no ENVBOX to fail closed, so that partial infrastructure never silently disables virtualization.
37. As a developer, I want Architecture (x64/x86) reported by Capability Probe, so that the correct runtime image is attached without guessing.
38. As a user, I want Integrity Level mismatches (e.g. cannot open higher-IL process) rejected with a clear reason, so that elevation problems are not misread as Profile bugs.
39. As a developer, I want Session Registry to be the authority for PID → Session/Profile after bootstrap, so that Environment Variables are no longer the source of truth at runtime.
40. As a user, I want the product story to stay “one Environment View for a process tree”, not “one more isolation box”, so that scope stays on environment consistency.

## Implementation Decisions

- Vocabulary stays aligned with the domain glossary: **EnvironmentSession**, **LaunchTarget**, **ActivationBackend**, **AttachStrategy**, **TargetCapabilities**, **IsolationGuarantee**, **Runtime IPC Bootstrap**, **Injection Support**, **early-start race**, **Startup Fail Policy**, **Process-scoped / Host-transparent / Environment-consistent**. Plan synonyms map as: *ActivationType* → **LaunchTarget**; *ProcessCapability* → **TargetCapabilities** + **Injection Support** decision; *Broker* → Host side of **Runtime IPC Bootstrap**.
- Control-plane pipeline (stable order): Target resolve → Capability probe → Backend selection → Activation → Attach → Bootstrap → Track. No `if windowsapps` outside capability/backend selection.
- **EnvironmentSession** is the control-plane aggregate for one Run. It carries: id, application id, LaunchTarget, profile id, attach strategy, isolation guarantee, root process set, process set, optional Package Identity, session state. Persisted run records may continue to use the RuntimeInstance name until storage migration; semantics are session-first.
- **ActivationBackend** produces an activated process (live PID, optional package identity, suspended-or-running state, capability snapshot). Implementations: Win32 (and Command via resolution) and Packaged (AUMID). Attach is a separate contract (RuntimeAttacher / RuntimeInjector) shared by all backends.
- **AttachStrategy** selection from capabilities: can_suspend + can_inject → PreExecution; else can_inject → PostActivation; else unsupported. IsolationGuarantee follows: FullPreExecution / PostActivation / Partial (reserved PackageDebug).
- Win32 backend behavior is locked: CreateProcess suspended → attach Runtime → handshake → resume. `caller_requested_suspended` still means do not auto-resume. Environment Block remains valid for Win32 (including ENVBOX_* fallback and inherit flags).
- Packaged backend: only `IApplicationActivationManager::ActivateApplication(AUMID)`; never launch a WindowsApps exe as root. No Environment Block. IsolationGuarantee = PostActivation (documented early-start race). Child processes of a supported packaged root still use pre-execution attach where possible.
- **Capability Probe** (process-level, before inject): process type / packaging, architecture, integrity level, AppContainer, mitigation policies (signature, dynamic code, image load). Policy (no bypass): AppContainer → Unsupported; Protected Process → Unsupported; MicrosoftSignedOnly/StoreSignedOnly or unknown blocking mitigations → Unsupported; cannot open/query process → Unsupported; mediumIL packaged Win32 without blocking mitigation → Supported. Failure is Fail Closed at session start.
- **Broker (`envbox-broker`)** is the Host authority for Session Registry and PID → Profile. It accepts Runtime IPC Bootstrap connections and process-lifecycle notices. V0.3 requires a real broker process in the preferred path; in-process HostBroker may remain as a transitional implementation behind the same protocol, but the protocol and session registry must be broker-shaped.
- **Runtime configuration channel (preferred):** DLL init → PID → Named Pipe connect to Broker → HELLO → GET_PROFILE → PROFILE (RuntimeProfile DTO) → install hooks. Message contract (stable names): HELLO, GET_PROFILE, PROFILE, PROCESS_CREATED, PROCESS_EXITED, RUNTIME_READY, HOOK_ERROR. Wire format is the existing line protocol unless a later ticket deliberately versions it; receivers ignore unknown keys/messages.
- **Fallback channel:** `ENVBOX_PROFILE_ID` / related ENVBOX_* remain a Win32 fallback when Broker is unavailable. Order after cutover: prefer Broker PROFILE; if Broker fails and ENVBOX_* is present, fall back; if both fail, Startup Fail Policy (never silent unvirtualized).
- **Remove C++ TOML parsing.** Rust owns Profile load/validate (serde). Runtime C++ holds a structured RuntimeProfile (timezone, locale, language, dns, registry, inherit/audit flags, ids) filled only from IPC DTO or a minimal ENVBOX-compatible path that does not parse `profiles.toml`. C++ does not read config files, does not parse TOML, does not validate profile business rules. This is a deliberate simplification of the Runtime contract, not a feature flag.
- **Child process upgrade:** keep CreateProcess* hook suspend → attach → resume behavior. Additionally notify Broker (parent PID, child PID) and let Broker assign session membership. Session membership: root descendants OR (package family match AND creation within activation window). Never exe-name-only matching.
- **Process Tracker** unifies lifecycle: Job Object tracker for Win32 (best-effort kill/stop/stats) and Package/PID tracker for Packaged. One session stop path walks the tracked set. Job Object is not a security boundary.
- Module layout (logical; exact crate boundaries may follow workspace conventions): core session/capability contracts; launcher activation (win32 / package) and attach (injector / runtime); broker session registry + ipc + process tracking; package discovery; runtime C++ hooks + ipc client + profile DTO consumer. Prefer evolving existing modules over a big-bang rewrite; extract crate boundaries when they reduce coupling (especially Broker and Package discovery).
- GUI/CLI share the same session start/stop API. UI redesign stays out of this spec.
- Fail Open remains for non-critical hooks. Fail Closed remains for cannot-activate / cannot-attach / cannot-resolve-profile / unsupported capability.
- Hook modules (time, geo, locale, language, dns, registry, process) and their virtualization semantics are not redesigned here. Audit recording stays compatible with the existing JSONL event schema unless a separate ticket versions it.
- Delivery is phased to avoid a flag-day (behavior lock each phase):
  1. **Phase 1 — Abstraction:** EnvironmentSession / ActivationBackend / AttachStrategy contracts; Win32 behavior unchanged; Probe + run acceptance green.
  2. **Phase 2 — Broker IPC:** Runtime ↔ Broker protocol and Session Registry live; keep ENVBOX_* fallback.
  3. **Phase 3 — Config cutover:** Runtime prefers Broker PROFILE; fallback ENVBOX_*; delete C++ TOML parser.
  4. **Phase 4 — Package Backend:** AUMID activation + post-activation attach; test with Mimo / Windows Terminal / a Store app as available.
  5. **Phase 5 — Hardening:** Capability Probe completeness, Package Tracker, Audit/session integration, docs for Tier 1/2/3 and early-start race.

## Testing Decisions

Good tests assert **external behavior** at the highest existing seams. They do not assert Detours helper internals, pipe byte layouts beyond public message names, C++ parse helpers, or hook call counts as product contracts.

Confirmed seams for this spec (user-accepted):

1. **Primary product path (existing):** `envbox run` (or the shared session start API used by CLI/GUI) **+ Probe output + Host snapshot unchanged**. This is the golden path for Win32, and later for Broker-transparent and Packaged runs. C++ TOML removal is a regression of the same Probe assertions under the same Profile — no new seam.
2. **Capability policy table (existing secondary):** pure rules for process type / integrity / AppContainer / mitigation → `can_inject` + reason. Unit-testable without a Store app. Prior art: packaged-v1 capability policy tests in core.
3. **IPC / Broker handshake (existing secondary):** protocol round-trip with a fake broker (hello → profile payload; process lifecycle messages accepted). Prior art: packaged-v1 IPC round-trip / FakeBroker tests. Used for Broker extraction and the Broker-first profile path.

What each phase must keep green:

- Phase 1: full existing run + Probe matrix (Win32 golden path unchanged); capability and session unit tests.
- Phase 2/3: same Probe matrix with Broker preferred path; Win32 with Broker down + ENVBOX_* still virtualizes; Broker down + no ENVBOX fails closed; C++ no longer reads `profiles.toml` (behavior proven by Probe under Broker, not by asserting the absence of parse helpers).
- Phase 4: packaged acceptance (machine-conditional, real mediumIL app when present): AUMID activate → PID → capability supported → Runtime loads → Probe shows Profile values → child pre-start attach where possible → Host unchanged → stop leaves package lifecycle clean. Unsupported packaged (AppContainer / signature-blocked) fails closed with reason.
- Phase 5: session tracking/stop, Audit still records, persistence fields present.

Do not expand verification into reverse-engineering Detours ordinals, full registry sandbox checks, or GUI snapshot testing for this feature.

## Out of Scope

- Sandbox / VM / driver-level isolation / WFP driver
- Anti-detection / hiding EnvBox / modifying Host Locale / Region / Timezone
- Font virtualization
- WebRTC hooks
- AppContainer / UWP / protected-process virtualization (remain Unsupported, Fail Closed)
- PSF unpack/re-sign or third-party package rewrite
- Eliminating packaged root early-start race
- Changing Win32 pre-execution injection contract or `caller_requested_suspended` semantics
- GUI redesign (UI frozen for this feature)
- Hot-reload Profile into a live session
- Replacing Detours / rewriting hook modules
- Full Registry Sandbox (Registry Virtual View remains whitelist-path virtualization)
- New isolation tiers beyond the documented Tier 1/2/3 model

## Further Notes

**Relationship to prior work.** packaged-v1 already introduced EnvironmentSession, ActivationBackend, AttachStrategy, TargetCapabilities, IsolationGuarantee, Runtime IPC Bootstrap names and a Packaged activation direction. V0.3 does not redesign Aura; it **finishes the platform cut** those types imply: Broker as the real config/session authority, C++ TOML removal, Process Tracker dual backend, and phased WindowsApps delivery. Keep the already-validated Runtime hooks.

**Product identity.** DLL injection is AttachStrategy #1, not the product. The claim stays: different Windows application models, one Environment Session Runtime / one Environment View per process tree.

**Tier model (docs, not code).** Tier 1 native Win32 full pre-exec; Tier 2 Packaged Win32 mediumIL post-activation; Tier 3 AppContainer / protected unsupported. Do not promise one injection technique covers all Windows processes.

**Why Broker now.** File-based `profiles.toml` + C++ parse + ENVBOX_* cannot express packaged roots or multi-process session membership. A PID-keyed Session Registry over Named Pipe is the minimum structure that lets Win32, Packaged, and later Sandbox/WSL backends share one Runtime bootstrap.

**Why delete C++ TOML.** Dual parsing is a correctness bug farm. Rust serde already validates Profile; shipping a second parser into every target process multiplies failure modes and blocks DTO evolution. After cutover, “Profile is wrong” has one owner.

**Seams check (accepted).** `envbox run` + Probe remains the only product-level acceptance ladder. Capability policy and Broker handshake stay secondary and narrow. If a change seems to need a fourth seam, prefer folding it into Probe output or the handshake contract.

**ADR note.** No ADR tree is present today. If Broker process lifetime or IPC wire format needs a durable decision record later, capture it as an ADR under `docs/adr/` when implemented — not as a silent convention in code.
