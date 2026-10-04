# 06: 持久 Container 创建编辑与版本化保存

Stage: P1
Status: claimed
Blocked by: None (can start immediately)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 用户通过 GUI/CLI 创建和编辑命名工作区，重启后稳定 UUID、Profile 引用和模式保留，旧配置继续可用。

## 负责模块与契约

Core、Storage、GUI/CLI 工作区最小管理。Container 为持久聚合，EnvironmentSession 仍为一次 Run；使用 TOML schema 和原子保存。

## 不包括

不启动应用、不复制 Profile、不隔离 AppData 或 Registry；本票不提供删除私有数据。

## 验收标准

- [ ] 创建 A/B 后重启读取 UUID、名称、Profile 引用和 schema 一致；重名不导致身份混淆。
- [ ] GUI 与 CLI 编辑同一对象，保存失败不破坏原配置。
- [ ] 旧 Application/Profile 数据无需重建且内容不被迁移覆盖。
- [ ] 损坏、未知 schema、无效 Profile 引用或不支持模式明确报错，不静默降级。
- [ ] Compatibility 为当前默认；目录仅 Aura 元数据，不展示 Overlay 已启用。

## 验证证据

配置 roundtrip、原子写失败和旧数据 fixture；最小 GUI/CLI 行为对照。

## 关联验收

A01、A13、A14、F10。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
