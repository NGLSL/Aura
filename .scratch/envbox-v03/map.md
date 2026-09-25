# envbox-v03 map

Spec: [spec.md](./spec.md) · Status: ready-for-agent

| # | Ticket | Phase | Blocked by |
|---|--------|-------|------------|
| 42 | [Session 抽象收敛与 Win32 行为锁](./issues/42-session-contracts-win32-lock.md) | 1 | — |
| 43 | [Broker 进程与 Session Registry](./issues/43-broker-session-registry.md) | 2a | 42 |
| 44 | [Runtime IPC 握手接入 Broker](./issues/44-runtime-ipc-bootstrap.md) | 2b | 43 |
| 45 | [Broker 优先 + 删除 C++ TOML](./issues/45-profile-cutover-drop-cpp-toml.md) | 3 | 44 |
| 46 | [Package Backend 端到端](./issues/46-packaged-backend-e2e.md) | 4 | 45 |
| 47 | [Process Tracker 与会话归属](./issues/47-process-tracker-session-membership.md) | 5a | 46 |
| 48 | [Capability + Audit + Tier 文档](./issues/48-capability-audit-tier-docs.md) | 5b | 46 |

依赖主链：42 → 43 → 44 → 45 → 46 → {47, 48}。

与 packaged-v1（37–41）关系：方向同源；V0.3 在已落地代码上完成 Broker 化、去 C++ TOML 与 Packaged 端到端，控制面以本 spec 为准。

## Review 残留（2026-09-27）

已修：Broker 先于 attach 启动；Packaged 走默认 pipe；LoadLibrary 退出码 Fail Closed；PROFILE inherit/audit 实值。

未闭环：
- `envbox-runtime*.dll` 需 MSVC/CMake 重建后重跑 Probe（本机无 cl.exe）
- GUI `InstanceManager` 仍走 `launch()`，CLI 已用 `start_session()`
- `envbox-broker.exe` 未被 CLI 自动拉起（进程内 HostBroker 为过渡）
- 真实 Store 应用 e2e（Mimo / Terminal）机测待跑
- `Profile.environment` 自定义变量未进 IPC DTO（Packaged 场景）
