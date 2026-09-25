Parent: .scratch/envbox-v03/spec.md

# 47: Process Tracker 与子进程会话归属（Phase 5a）

**What to build:** 统一生命周期：Job Tracker（Win32）+ Package/PID Tracker（Packaged）；stop/状态/审计读同一会话进程集。

**Blocked by:** 46

**Status:** resolved

- [x] Process Tracker 抽象：登记 root/child，Job Object 与 Package/PID 双轨实现
- [x] `belongs_to_session`：root 后代 **或** PackageFamilyName 匹配 + activation 窗口；禁止仅 exe 名匹配
- [x] session stop：按 tracker 终止/通知进程集；Win32 继续 Job 一键停止；Packaged 不改 package 配置/debug 策略
- [x] 异常退出与 stale PID 清理；Session 状态可观察
- [x] 多进程树（Electron/Broker/COM 伴生）不丢会话边界（ancestry + package window）
- [x] 验收：stop 后无残留虚拟化进程；Host/package 不被污染（既有 boundary 测试覆盖 Job 路径）

## Comments

### 2026-09-27 done

新增 `process_tracker`：`TrackMode::{Job, PackagePid}`、parent-aware `register_child`、package window `belongs`。`SessionHandle` 携带 tracker。IPC SessionRegistry 同步维护 live PID 集。
