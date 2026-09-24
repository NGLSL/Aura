Parent: .scratch/envbox-v02/spec.md

# 22: Audit Mode — CLI 查询/导出

**What to build:** `envbox audit show <instance_id>` 与 `envbox audit export`。

**Blocked by:** 20 Audit Mode — schema + sink

**Status:** resolved

- [x] `envbox audit show <instance_id>`：打印该实例 JSONL（或摘要表）
- [x] `envbox audit export [--out path]`：导出/合并审计文件
- [x] 未知 instance_id / 缺文件：明确错误，非 0 退出码
- [x] 解析使用 schema v1 公共契约；单测覆盖合法/非法行
- [x] usage 文案更新

## Comments

### 2026-09-24 implementation

- `audit show` 输出原始 JSONL；`--summary` 按 API 聚合 calls/virtualized
- `audit export` 合并 `audit/*.jsonl` 到 stdout 或 `--out`；空目录/非法行 fail closed
- 测试：`crates/envbox-cli/tests/cli_audit.rs`（show/summary/missing/bad-line/export/empty）
