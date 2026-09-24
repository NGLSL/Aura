Parent: .scratch/envbox-v02/spec.md

# 34: 边界 — 异常退出 / Job Object

**What to build:** 崩溃、Terminate、Job close 无句柄泄漏；终态正确。

**Blocked by:** （无）

**Status:** open

- [ ] 异常退出后 InstanceStatus ∈ Exited/Failed
- [ ] 进程/线程/Job 句柄 RAII，无泄漏
- [ ] `KILL_ON_JOB_CLOSE` Stop 语义保持
- [ ] 自动化覆盖

## Comments
