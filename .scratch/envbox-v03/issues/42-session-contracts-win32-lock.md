Parent: .scratch/envbox-v03/spec.md

# 42: EnvironmentSession 抽象收敛与 Win32 行为锁（Phase 1）

**What to build:** 在已落地的 Session/Activation/Attach 类型上完成控制面收敛，确保 Win32 PreExecution 金路径语义不变，并把一次 Run 明确建模为 EnvironmentSession。

**Blocked by:** （无；本系列入口，先锁行为）

**Status:** resolved

- [x] EnvironmentSession 成为一次 Run 的控制面聚合：LaunchTarget、profile_id、attach_strategy、isolation_guarantee、root_processes、processes、package_identity?、state
- [x] ActivationBackend 产出 ActivatedTarget；RuntimeAttach 独立完成注入；Win32 仍 `CreateProcess(SUSPENDED) → attach → resume`（`caller_requested_suspended` 不得自动 Resume）
- [x] AttachStrategy / IsolationGuarantee 选择规则稳定：can_suspend+can_inject → PreExecution；仅 can_inject → PostActivation；否则 Fail Closed
- [x] LaunchTarget 覆盖 Win32 / Command / Packaged；serde 兼容既有 `executable` / `command` 文档
- [x] 持久化 run 记录可表达 session 字段（`RuntimeInstance::from_session` 映射 isolation/attach/package）
- [x] `envbox run` + Probe 验收矩阵保持全绿（Host 不变；禁止静默无虚拟化）

## Comments

### 2026-09-27 kickoff

Phase 1 只抽象、不改行为。与 packaged-v1 #37 同向；本票以当前代码为基线做收敛与行为锁，而不是从零重写。Hooks 不动。

### 2026-09-27 done

新增 `ActivationType`、`RuntimeInstance::from_session`、session 字段映射测试。`cargo test --workspace` 全绿（含 run+Probe 矩阵）。
