# Status: ready-for-agent

## Problem Statement

V0.1 已证明 Process-scoped environment virtualization 可行：真实应用（Mimo 客户端）及其子进程在不改 Host 的前提下读取到一致的 Profile 环境视图。但 V0.1 覆盖率、一致性和可观测性仍不足：

1. **看不到目标程序实际读了什么** — 不知道 Claude / Codex / Mimo 等代理客户端调用了哪些地域相关 API，无法判断 Hook 覆盖缺口，也无法用证据指导下一轮拦截。
2. **DNS View 与真实解析不一致** — `GetNetworkParams` / `GetAdaptersAddresses` 返回 Profile 服务器，但解析流量仍可能走 Host DNS，出现「看到 1.1.1.1、实际被宿主解析」的逻辑矛盾，破坏 Environment-consistent。
3. **边界场景未验证** — 提权子进程、x86 目标、多级 `.cmd` wrapper、Electron 多进程、异常退出/Job、`caller_requested_suspended` 等仍是 residual 或空白，真实客户端长期运行时不稳。
4. **体验未收口** — GUI 错误反馈、快捷方式、启动延迟（release）等影响日常使用。

V0.2 不扩大虚拟化范围（仍不做字体、WebRTC、硬件指纹、防检测），而是把已立住的核心链路做**可审计、一致、可长期使用**。

## Solution

在 V0.1 冻结架构上交付四块：

1. **Audit Mode** — Runtime 在 Fail Open 语义下旁路记录进程树对地域相关 API 的调用（API 名、是否命中虚拟化、摘要值），按 RuntimeInstance 落盘 JSONL；CLI 可查询/导出。默认关闭，关闭时零写盘、行为与 V0.1 完全一致。
2. **DNS per-process routing（最小闭环）** — 在 DnsMode `VirtualView` 下，对进程内解析入口（`DnsQuery*` / `getaddrinfo` 等）按 Profile 指定的 DNS servers 做 per-process 解析路由；不做驱动、不做透明劫持、不影响其它进程。
3. **边界兼容性** — 按风险补测并修复：提权/完整性级别、`envbox-runtime32` x86 注入、多级 cmd/bat wrapper、Electron 多进程树、异常退出与 Job 句柄、`caller_requested_suspended` 自动化。
4. **使用体验收口** — GUI 错误提示与列表稳定性、快捷方式/Run With 打磨、release 启动延迟复测。

隔离单位仍是 **Process Tree Instance**；Host 仍完全不被修改；Startup Fail Policy / Fail Open / Profile per-instance immutable 契约不变。

## User Stories

1. As a developer, I want an Audit Mode switch (default off) on a Run, so that I can record which locale/region/timezone/DNS APIs a tool actually queries without enabling it always.
2. As a developer, I want each audit event to record API name, process/thread ids, timestamp, whether virtualization applied, and a short original/returned value summary, so that I can see coverage gaps without reading secrets.
3. As a user, I want audit output written per RuntimeInstance as JSONL under EnvBox’s data directory, so that multiple concurrent runs never mix trails.
4. As a user, I want child processes in the Process Tree Instance to append to the same instance audit file, so that a full spawn tree is reconstructible via pid/ppid.
5. As a developer, I want `envbox audit show` and `envbox audit export` to read those files, so that I can answer “what did this agent query?” from the CLI.
6. As a user, I want Audit Mode off to produce no audit file and identical API behavior to V0.1, so that observability never changes virtualization semantics.
7. As a user, I want audit summaries to exclude file contents, tokens, full environment blocks, and command bodies, so that trails stay safe to share.
8. As a developer, I want Audit Mode to prove which unhooked geo/locale/timezone APIs are still hit, so that V0.3 Hook expansion is evidence-based.
9. As a user, I want DNS Mode `VirtualView` to route name resolution from the process tree to the Profile DNS servers, so that what the app sees and what it resolves stay consistent.
10. As a user, I want resolution hooks to cover the common client entry points (`DnsQuery_A/W/UTF8/EX`, `getaddrinfo`, `GetAddrInfoW/Ex` as prioritized), so that typical Windows/.NET/Node/Python resolvers honor the Profile.
11. As a user, I want DNS routing failures to Fail Open to the original resolver, so that a bad Profile DNS entry does not break name resolution entirely.
12. As a user, I want DnsMode `Host` to leave resolution completely untouched, so that Host-only profiles remain transparent.
13. As a user, I want non-VirtualView processes and the rest of the OS to keep using Host DNS, so that routing is process-scoped like all other virtualization.
14. As a developer, I want Probe to show resolution outcome contrast under VirtualView vs Host (e.g. a controlled test name or documented contrast method), so that DNS routing is acceptance-tested.
15. As a user, I want elevated / high-integrity child processes either injected correctly or rejected with a clear Startup Fail Policy message, so that I never assume virtualization that did not happen.
16. As a user, I want x86 targets to load `envbox-runtime32.dll` and virtualize like x64, so that 32-bit tools in the tree keep the Profile.
17. As a user, I want multi-level `.cmd`/`.bat` wrappers (wrapper → node → child) to keep the Profile at every level, so that npm-style CLIs remain consistent.
18. As a user, I want Electron-style multi-process trees (main, renderer, utility) to inherit the Profile, so that GUI agents built on Electron stay in one Environment View.
19. As a user, I want abnormal exit, Terminate, and Job close to leave no leaked process/thread handles and a final InstanceStatus of Exited or Failed, so that long sessions stay clean.
20. As a developer, I want automated tests for `caller_requested_suspended`, so that debuggers/custom launchers keep suspension control after injection.
21. As a user, I want GUI Save/Run failures to show the actual error reason, so that I can fix Path/Profile/DLL issues without reading logs first.
22. As a user, I want Application list selection to survive refresh, so that editing and Run With are not reset by background updates.
23. As a user, I want release-build launch latency re-measured against ideal &lt; 100 ms / acceptable &lt; 300 ms, so that the performance target is honest for shipping builds.
24. As a user, I want README/About to keep the non-security-boundary disclaimer and the V0.2 product line, so that scope is never oversold.

## Implementation Decisions

- Vocabulary continues to follow `docs/CONTEXT.md`. New terms must be added there before implementation: **Audit Mode**, **Audit Event**, **DNS routing** (distinct from **DNS View**).
- DnsMode remains `Host` / `VirtualView` only. DNS View stays “what config APIs return”; DNS routing is “where the process tree resolves names” under `VirtualView`.
- Audit Mode is a Run-scoped or Profile-scoped flag (default **off**). Prefer Application/Profile field `audit` plus CLI `--audit` override; effective value is copied into `ENVBOX_AUDIT=1` and the immutable RuntimeProfile at launch — no hot reload.
- Audit sink: `%LOCALAPPDATA%\EnvBox\audit\<instance_id>.jsonl` (or `ENVBOX_CONFIG_ROOT\audit\`). Append-only; one file per RuntimeInstance; children inherit the path via RuntimeProfile/env. No shared global file.
- Audit Event fields (stable schema, versioned): `v`, `ts_utc`, `pid`, `ppid`, `tid`, `api`, `virtualized` (bool), `note`/`summary` (short, non-sensitive). Never log file contents, full env blocks, tokens, or arbitrary memory.
- Audit writes are best-effort and non-fatal: sink failure must not break hooks (Fail Open). Lock-light append; no polling; no timers.
- Hook audit points stay inside existing domain modules (`hooks_time`, `hooks_geo`, `hooks_locale`, `hooks_language`, `hooks_dns`, `hooks_registry`, `hooks_process`) via a small `audit.h` helper — do not create a second hook layer.
- DNS routing (V0.2 minimal): implement as additional hooks in `hooks_dns.cpp` for prioritized resolver APIs. Under `VirtualView`, query Profile `servers` in order; on total failure Fail Open to original API. Do **not** implement WFP, LSP, DoH interception, raw port-53 redirection, or system-wide proxy.
- DNS routing acceptance must not depend on public Internet flakiness alone: prefer a local/fixture resolver or documented contrast (e.g. different NXDOMAIN/answer for a test name) plus Host-unchanged checks.
- x86: build `envbox-runtime32.dll` beside 64-bit artifact; launcher already prefers arch-matching DLL names. x86 injection follows the same `DetourCreateProcessWithDllExW` path.
- Elevation: if injection into a higher-integrity child is impossible, Startup Fail Policy applies (reject / fail the create path that would silently unvirtualize). Document required integrity level in README; no silent degrade.
- `caller_requested_suspended`: keep V0.1 semantics (inject, do not Resume). Add Probe/CLI automation (e.g. a tiny helper that CreateProcess with `CREATE_SUSPENDED` and asserts still-suspended after inject).
- Job Object remains lifecycle-only (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`); abnormal exit paths must close handles (RAII in Rust launcher; saved `GetLastError` + closed PI handles in C++ spawn paths — already enforced in code review).
- GUI remains thin over core/launcher. V0.2 GUI work is error surface and list stability only; no injection logic in UI. Optional shortcut integration is shell-level and Host-transparent (does not modify Host system settings).
- Logging remains `tracing` (Rust) + Runtime file log at INFO; Audit Mode is separate from Debug hook-hit logs (story 55 in V0.1). Audit is structured JSONL for analysis; Runtime log is operational.
- Performance: Audit off must not regress launch latency vs V0.1. Audit on may cost append I/O but stays event-driven (no polling).
- Security posture text unchanged: EnvBox does not provide a security boundary.

## Testing Decisions

- Same test philosophy as V0.1: assert external behavior at Probe / CLI / public library seams only. Do not assert Detours internals or hook call counts as product contracts.
- **Audit seam (new, primary for priority 1):**
  - off → no `audit\<instance_id>.jsonl` created; Probe snapshot identical to V0.1 expectations.
  - on → file exists, is valid JSONL, contains expected API names when Probe exercises them (timezone/geo/locale/language/dns), and virtualized flags match known Profile vs Host cases.
  - child inheritance: parent + `--spawn-child` events share one instance file with pid/ppid linkage.
  - negative: no secrets-shaped fields; sink directory unwritable ⇒ run still succeeds (Fail Open).
- **DNS routing seam:**
  - `Host` mode: resolution path not overridden (behavior matches Host).
  - `VirtualView`: controlled name resolution reflects Profile servers (deterministic fixture preferred).
  - Fail Open: unreachable Profile servers eventually succeed or fail the same way as original API when falling back — never hang forever without error.
  - Host DNS and other processes unchanged after runs.
- **Boundary seams:** one focused test per row in the compatibility matrix (elevation, x86, multi-level cmd, Electron-like multi-child, abnormal exit/Job, `caller_requested_suspended`).
- **GUI last:** smoke only (error message surfaces, list selection stability). Not a substitute for Probe.
- Acceptance matrix extension: keep V0.1 tools (`envbox-probe`, `cmd`, `powershell`, `git`, `node`, `python`, `notepad`, real Node CLI agent) and add Audit on/off pair + one DNS contrast case + one x86 case when runtime32 lands.
- Host transparency remains mandatory after the whole matrix.
- Unit tests allowed only where Probe cannot isolate cheaply: audit JSON schema serialize/parse, DNS server selection order, CLI argument parsing for `envbox audit`.

## Out of Scope

- Fonts, WebRTC, hardware fingerprint, disk/BIOS/GPU serial, MAC spoofing, user SID virtualization
- Anti-debug, hidden injection, anti-detection, stealth
- Full Registry sandbox, filesystem isolation, VM / Windows Sandbox / Hyper-V backends
- WFP drivers, LSP, transparent packet redirection, DoH interception, system-wide or per-app VPN
- Public IP / account region changes; browser fingerprint spoofing
- Hot-reloading Profile for a live RuntimeInstance
- Strict Mode / sandboxed backends (later)
- Formatting API virtualization expansion beyond what V0.1 already reserved
- Multi-user / service-session injection; non-Windows hosts

## Further Notes

- Preserve the three principles in every change: **Process-scoped**, **Host-transparent**, **Environment-consistent**.
- Implementation order: Audit Mode → DNS routing → boundary compatibility → UX. Do not start UX before Audit lands (Audit informs Hook gaps).
- V0.1 residuals explicitly owned by V0.2: x86 Runtime, `caller_requested_suspended` automation, ideal &lt; 100 ms latency re-measure, elevation messaging, multi-level wrapper/Electron coverage.
- Issue files live at `.scratch/envbox-v02/issues/NN-*.md` with `Status:` lines; comments under `## Comments`.
- V0.1 remains frozen: no new virtualization domains without a V0.2+ spec change.
