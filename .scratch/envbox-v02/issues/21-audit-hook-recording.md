Parent: .scratch/envbox-v02/spec.md

# 21: Audit Mode — Hook 旁路记录

**What to build:** 在现有 domain hooks 内旁路写 Audit Event，不改变虚拟化语义。

**Blocked by:** 20 Audit Mode — schema + sink

**Status:** resolved

- [x] `audit.h` 小助手：`EnvBoxAuditEvent` / `EnvBoxAuditEventW`
- [x] 覆盖：time / geo / locale / language / dns / registry / process hooks
- [x] 记录是否命中虚拟化与非敏感摘要（原值/返回值截断，如 locale/region/tz/langid hex）
- [x] Audit off 时零开销短路，行为与 V0.1 一致
- [x] 不记文件内容、token、完整 Environment Block、命令体
- [x] Probe 对照：on 时文件含预期 API 名；off 时无文件

## Comments

### 2026-09-24 implementation

**Evidence**
- 旁路点：`GetDynamicTimeZoneInformation`/`GetTimeZoneInformation`/`GetTimeZoneInformationForYear`、Tz 转换族 null-zone 路径、`GetUserDefaultGeoName`/`GetUserGeoID`、locale 名与 LCID、`GetLocaleInfoW`/`GetLocaleInfoEx`、UI language 与四个 PreferredUILanguages（真实 API 名）、`GetNetworkParams`/`GetAdaptersAddresses`、白名单 `RegQueryValueExW`/`RegGetValueW`（含 type-mismatch）、`CreateProcessW`（inject 成功/失败/suspended，无 cmdline）
- 摘要：locale/LCID/langid 输出真实 hex（`0x%08x` / `0x%04x`），不写占位符
- 集成断言：`run_audit_on_writes_instance_jsonl` 检查核心 API + `GetNetworkParams` + `virtualized:true`
- `cargo test --workspace --exclude envbox-app` 全绿

### 2026-09-24 dual-axis review fixes

**Spec Wrong**
1. profile-langid/lcid 占位摘要 → 真实 `0x%04x` / `0x%08x`
2. `PreferredUILanguages` 合成名 → 四 wrapper 传真实 API 名
3. checklist 恢复「原值/返回值截断」
4. DNS `host-or-fail-open` → `fail-open` / `dns-host` / `dns-virtual-view`

**Spec Missing（本票范围内）**
- `GetLocaleInfoW`/`GetLocaleInfoEx` 重写时记录
- `GetTimeZoneInformationForYear` + Tz 转换 null-zone
- `CreateProcessW` inject-failure / no-runtime-dll / no-profile
- `RegGetValueW` type-mismatch
- PreferredUILanguages size-query / fail-open / insufficient-buffer 中途路径
- 测试断言 `GetNetworkParams`
- （spawn-child 共文件、不可写 sink 等留给 ticket 23）

**Standards Hard**
- `SetLastError` 后 audit 可能污染 `GetLastError` → `EnvBoxAuditEvent`/`W` 入口保存并在返回前恢复

**Standards Judgement**
- DNS token 拆分；PreferredUILanguages 真实 API 名；宽摘要溢出改为截断而非整段丢弃；`inject-resume-failed` 记 `virtualized:0`
