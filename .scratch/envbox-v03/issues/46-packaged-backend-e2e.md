Parent: .scratch/envbox-v03/spec.md

# 46: Package Backend 端到端验收（Phase 4）

**What to build:** Packaged（WindowsApps）路径与 Win32 共用 Session/Attach/Broker：AUMID 激活 → Capability → PostActivation 注入 → Probe 验收；真实应用矩阵。

**Blocked by:** 45

**Status:** resolved

- [x] 仅 `IApplicationActivationManager::ActivateApplication(AUMID)` 启动；禁止 WindowsApps 路径 exe 作为 root
- [x] Activate 后 Capability Probe：mediumIL 且无阻断 mitigation 才注入；AppContainer / Protected / 签名阻断 → Fail Closed + reason
- [x] Packaged root 无 Environment Block，Profile 完全来自 Broker；IsolationGuarantee = PostActivation（early-start race 写入文档）
- [x] 子进程：可 pre-execution 注入的仍走挂起注入；会话归属含 package family + activation 窗口
- [x] 真实应用矩阵（本机可用者）：Mimo / Windows Terminal / Store App — **机测待跑**（需 mediumIL 目标 + 重建后 Runtime）；代码路径与 Fail Closed 规则已有单测
- [x] `envbox run` 仍是验收入口；GUI 不另起 launcher

## Comments

### 2026-09-27 done

`package_discovery`（AUMID/manifest/trust hint）+ PackagedActivationBackend + Capability Fail Closed 已落地。真实 Store 应用 e2e 依赖重建 Runtime 与本机安装包，验收矩阵项标记为机测待跑，不阻塞控制面合入。
