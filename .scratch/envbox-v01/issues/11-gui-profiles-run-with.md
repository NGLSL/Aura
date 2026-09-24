Parent: .scratch/envbox-v01/spec.md

# 11: GUI — Profile 编辑 + Run With

**What to build:** 用户在 GUI 配置 Environment Profile 全字段并通过校验保存；可对 Application 使用 Run With 临时选择 Profile（含 Host），不修改默认 Profile。

**Blocked by:** 10 GUI — Application 管理 + Run/Stop

**Status:** ready-for-agent

- [ ] Profile 编辑：Region、Locale、UI Language、Timezone（Windows 枚举下拉）、DNS Host/Custom View、Environment Variables
- [ ] 保存前校验，不合法不得写入
- [ ] Run With 菜单列出 Profile 与 Host；临时 Profile 不覆盖 Application.default_profile_id
- [ ] 与 CLI 使用同一 storage/launcher 契约
