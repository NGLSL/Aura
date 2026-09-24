Parent: .scratch/envbox-v02/spec.md

# 21: Audit Mode — Hook 旁路记录

**What to build:** 在现有 domain hooks 内旁路写 Audit Event，不改变虚拟化语义。

**Blocked by:** 20 Audit Mode — schema + sink

**Status:** open

- [ ] `audit.h` 小助手：`EnvBoxAuditEvent(api, virtualized, summary)`
- [ ] 覆盖：time / geo / locale / language / dns / registry / process hooks
- [ ] 记录是否命中虚拟化与非敏感摘要（原值/返回值截断）
- [ ] Audit off 时零开销短路，行为与 V0.1 一致
- [ ] 不记文件内容、token、完整 Environment Block、命令体
- [ ] Probe 对照：on 时文件含预期 API 名；off 时无文件

## Comments
