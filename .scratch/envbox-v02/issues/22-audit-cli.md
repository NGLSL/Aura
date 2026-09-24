Parent: .scratch/envbox-v02/spec.md

# 22: Audit Mode — CLI 查询/导出

**What to build:** `envbox audit show <instance_id>` 与 `envbox audit export`。

**Blocked by:** 20 Audit Mode — schema + sink

**Status:** open

- [ ] `envbox audit show <instance_id>`：打印该实例 JSONL（或摘要表）
- [ ] `envbox audit export [--out path]`：导出/合并审计文件
- [ ] 未知 instance_id / 缺文件：明确错误，非 0 退出码
- [ ] 解析使用 schema v1 公共契约；单测覆盖合法/非法行
- [ ] usage 文案更新

## Comments
