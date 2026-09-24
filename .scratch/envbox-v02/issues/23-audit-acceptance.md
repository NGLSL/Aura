Parent: .scratch/envbox-v02/spec.md

# 23: Audit Mode — Probe/验收对比

**What to build:** Audit on/off 外部行为验收；证明可观测性不改变虚拟化。

**Blocked by:** 21 Audit Mode — Hook 旁路记录, 22 Audit Mode — CLI 查询/导出

**Status:** resolved

- [x] off → 无 audit 文件；Probe 快照与 V0.1 预期一致
- [x] on → jsonl 含 timezone/geo/locale/language/dns 相关 API；`virtualized` 标志正确
- [x] `--spawn-child` 与父进程共用 instance 文件且 pid/ppid 可关联
- [x] 目录不可写 ⇒ Run 仍成功（Fail Open）
- [x] 无密钥形态字段
- [x] Host 配置跑完矩阵后不变

## Comments

### 2026-09-24 implementation

- off/on 对照见 `cli_run.rs`（ticket 20/21）
- 新增 `cli_audit_acceptance.rs`：spawn-child 共文件 + pid/ppid、不可写 sink Fail Open、无密钥摘要
- Host 不变已有 `run_leaves_host_probe_snapshot_unchanged` + `acceptance_host_config_unchanged_after_matrix`
