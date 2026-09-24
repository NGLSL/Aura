Parent: .scratch/envbox-v01/spec.md

# 11: GUI — Profile 编辑 + Run With

**What to build:** 用户在 GUI 配置 Environment Profile 全字段并通过校验保存；可对 Application 使用 Run With 临时选择 Profile（含 Host），不修改默认 Profile。

**Blocked by:** 10 GUI — Application 管理 + Run/Stop

**Status:** resolved

- [x] Profile 编辑：Region、Locale、UI Language、Timezone（Windows 枚举下拉）、DNS Host/VirtualView、Environment Variables
- [x] 保存前校验，不合法不得写入
- [x] Run With 菜单列出 Profile 与 Host；临时 Profile 不覆盖 Application.default_profile_id
- [x] 与 CLI 使用同一 storage/launcher 契约

## Comments

### 2026-09-24 implementation + dual-axis review (general-11 → fix → general-12)

**Evidence**
- Profile 全字段 + IANA；Timezone 下拉 = `enumerate_dynamic_timezone_ids`；切换 TZ 时 `windows_id_to_iana` 预填，**未映射则清空**（禁止 Windows ID 假 IANA）。
- 保存前：`validate_profile` + 非法 DNS token **拒绝写入** + Env `KEY=VALUE;`（与 placeholder 一致）。
- Run With：菜单含 Host；`RunTarget::Host` → `LaunchRequest.profile=None` → plain CreateProcess、**无 Runtime 注入**、不设 `ENVBOX_PROFILE_ID`（真 Host，见 CONTEXT.md）。临时选择不改 `default_profile_id`（单测 `run_with_does_not_mutate_default_profile_id`）。
- 与 CLI 同契约：`envbox-storage::{validate_profile,enumerate_dynamic_timezone_ids,windows_id_to_iana}` + `envbox-launcher::{RunTarget,format_args,parse_args,launch}`。
- core：`looks_like_iana_id` 拒绝 `Pacific Standard Time` 之类 Windows ID。

**Review**
- general-11：Run With「含 Host」为 worst Spec break → 已修为真 Host。
- general-12 Hard=0；Wrong（stale IANA pair）→ 已修。

**Accepted V0.1 residuals**
- `windows_id_to_iana` 为小型静态表（`Option`，无 fallback）；未映射区需手填 IANA。
- Run With 菜单 `NamedId.id=Uuid::nil()` 仅作 UI 哨兵，领域上对应 `RunTarget::Host`。

### 2026-09-24 post-review hard/wrong (general-14/15 on 09e98ee..583f16d)

- Hard: checklist 术语 `Custom View` → `VirtualView`（CONTEXT.md）。
- Wrong: `looks_like_iana_id` 放行单段合法 IANA（EST/MST/CET/HST/GMT 等）。
