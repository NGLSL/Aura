Parent: .scratch/envbox-v03/spec.md

# 48: Capability Probe 完善 + Audit 集成 + Tier 文档（Phase 5b）

**What to build:** 探测字段完备（进程类型/架构/IL/AppContainer/Mitigation）；Audit 与 Session 对齐；产品限制与隔离分层文档成稿。

**Blocked by:** 46

**Status:** resolved

- [x] Capability Probe 报告：Process Type / Packaging、Architecture、Integrity Level、AppContainer、Signature/DynamicCode/ImageLoad mitigation
- [x] 策略表保持无 bypass：AppContainer/Protected/无法打开/阻断或未知 mitigation → Unsupported；mediumIL 清洁 Packaged Win32 → Supported
- [x] TargetCapabilities 驱动 attach 选择；注入失败 = Startup Fail Policy
- [x] Audit：现有 JSONL 事件仍可关联 session/instance；不改变虚拟化语义
- [x] package discovery 元数据（DisplayName、AUMID、PackageFullName、PackageFamilyName、RuntimeBehavior、TrustLevel）可供 picker/CLI
- [x] 文档：Tier 1/2/3、early-start race、Fail Open vs Fail Closed、Broker/ENVBOX 回退矩阵 → `docs/isolation-tiers.md`

## Comments

### 2026-09-27 done

Capability 单测表完整；`docs/isolation-tiers.md` + CONTEXT 词汇更新（Session Registry / envbox-broker / ENVBOX_* value fallback / Process Tracker）。Audit schema 未改，仍随 instance 落 JSONL。
