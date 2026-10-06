# 环境信息容器代码对齐与读取盘点

Date: 2026-10-06
Baseline: `40f81a5`
Branch: `dev`
Scope: 用户要求将实际功能按 Profile 环境信息容器规格调整；不实施存储/权限沙箱，不安装服务或驱动。

## R1 当前读取能力盘点

下表是当前源码与模型事实，不是所有应用入口的运行验收。

| 信息 | Profile 来源 / Runtime 路径 | 明确限制 |
| --- | --- | --- |
| Locale | locale.locale_name；hooks_locale、hooks_crt_locale | Windows 与 UCRT 已有入口，其他读取链路逐项验证 |
| UI Language / Region | locale.ui_language、region；hooks_language、hooks_geo | 不推断 WMI/COM/服务返回值全部被替换 |
| Timezone | timezone Windows/IANA ID；hooks_time、hooks_winrt_time | 本地时间换算虚拟化，UTC 时间线保持真实 |
| Environment | environment map、独立环境块、语言变量规则 | 程序自行修改环境变量有既有语义，不保证对应系统 API 一并变化 |
| Registry 信息读取 | registry.whitelist_paths；hooks_registry | 有限地域/时区/DNS 读视图，不虚拟化所有键值或 Registry 写入 |
| DNS | dns mode/strict/upstreams；hooks_dns、共享 query engine | 任意 QTYPE 与四传输；Host 保持 Windows DNS，strict 仅保证受支持路径无 Host fallback |
| 浏览器 / WebRTC | browser.webrtc；browser_policy、hooks_network、hooks_process | 浏览器政策与注入进程 guard；不能注入的 sandbox renderer 单独报告 Partial/Unsupported |
| Hostname/user/MAC/硬件身份 | 当前没有专门 Profile 字段 | 待明确读取需求后另行设计；不宣称支持 |

应用自主 DNS/DoH/DoT/DoQ、未注入进程、系统代执行、浏览器 renderer 都不能从主进程 Hook 安装数量推断覆盖。Profile 不改变实际公网 IP。

## 产品与保存行为

- GUI 容器页改为 Profile 信息视图、不可变快照和运行管理，移除存储规则编辑及 storage-policy-enforced 指标；对应编辑状态、消息和处理分支删除。
- CLI 不再公开 policy 编辑/预览；旧 `container policy` 在构造 ConfigStore/default_root 前明确拒绝，不创建目录、读取损坏文件或触发历史配置迁移。
- 现有 Compatibility mode 与持久化 schema 保持；Container/Strong 仍明确拒绝，原因指向当前信息容器范围而非缺驱动。
- 历史 storage_policy 保留在 Container 和快照摘要中，必要结构/schema 校验保留；移除保存和准备运行时的磁盘类型/跨卷资格依赖，不访问规则所指向的目录。
- GUI 保存只合并可编辑 name/profile_id，保留刚重读的模式、创建时间和历史 policy。成功后 draft/saved 同步实际保存对象；隐藏 metadata 不从旧草稿覆盖。
- 修复过时 DoH 未启用文案、全局 Hook 安全回退说法和 Host/VirtualView 说明；strict DNS 与多数 Hook 的兼容回退分开描述。

## 实际 Runtime 安装事实

复用已有 Broker 对 RuntimeIdentity 的身份、Profile、配置和 Hook 校验，不新增 Runtime 协议。Supervisor 的 RunResult 与 MemberRuntimeIdentity 增加 optional environment_facts：config_complete、profile_matches_snapshot、每组 Hook 的 attached_api_count。

这些字段只从已通过验证的实际 observation 采集。旧记录缺字段为 None/unknown，schema 2 候选不补造已验证事实。恢复重新连接后重新采集；按 PID 与 creation_time 区分根进程和成员，根已退出时不把子进程事实放在根字段。未改变 RunSnapshot schema/digest。

GUI/CLI 显示配置匹配、Hook 安装和已记录成员事实，同时明确安装事实不是 API 行为验收，不表示整棵进程树的信息都已隔离。PE entry gate 不涵盖 import DLL/TLS initializer 的已知时序边界保留。

## 验证

- Storage containers：11 passed；历史跨卷规则不阻塞保存/改名/快照、完整保留 metadata；不创建目标卷目录。
- CLI 容器/配置/DNS 配置定向：10 passed；旧 policy 四类命令无创建/修改副作用。
- Supervisor：19 passed / 0 failed / 16 ignored；包括旧字段缺失、真实安装事实结构持久化、PID generation 与 root exit。
- App check 和既有 workspace state 测试通过；审查补强后的 state 测试实际覆盖 GUI 打开后外部更新同容器 metadata，再改名保存的保留行为。
- Rust 1.99 locked workspace build/test exit 0：455 passed / 0 failed / 33 ignored；fresh WMI controller 9548 / Runtime 0，`target/workspace-environment-view-{build.log,test.log,result.json}`。该全量发生在最终 GUI 两处 P2、Supervisor 文案/测试清理补丁之前；补丁后分别做对应定向检查，不把先前全量冒充修后验证。
- Runtime C++ 与冻结 pair 本轮未改；沿用 `target/service-bootstrap-runtime/` 双架构实际产物。原生启动与恢复证据单独记录，ignored 不计为通过。
- 原生新增事实整链：fresh WMI controller 29536 / Runtime 0；实际不可变 Run 1/1、同架构崩溃恢复 1/1、混合架构恢复 4/4，均 exit 0。实际断言配置完整/Profile匹配/已附加DNS Hook及 fresh 恢复事实，Stop A 保留 B/Host，覆盖64→32、32→64以及root存活/退出。`target/environment-view-native-result.json` 与对应 `-run/-recovery/-mixed` stdout/stderr 保存原始证据。这是 approved fixture server 运行，不是 GUI 手工验收。
- 最后补丁与格式整理后，fresh WMI controller 468 / Runtime 0 的 Rust 1.99 locked workspace build exit 0，`target/environment-view-final-build{.log,-result.json}`；最终可执行文件已包含补强，未只停留在 check 结果。
- 变更 Rust 的 rustfmt 检查通过，settings.rs 除外：该文件未修改位置有既有格式差异，已对 `40f81a5` 原文件独立确认相同问题，`target/settings-format-baseline.log`；未为此次文案调整格式化无关代码。git diff check 通过。

## Standards

初审 P2：GUI 隐藏 policy 后仍可能用旧 draft 覆盖外部刚更新的 metadata。已改为只合并编辑字段并加现有状态测试场景，增量复核 0 剩余 actionable findings。

## Spec

初审 P2：Host Profile 也无条件显示 Profile DNS 上游保证。已区分 Host 与 VirtualView 条件，增量复核 0 剩余 actionable findings。

## 未验收项

实际 GUI 布局/交互、浏览器 renderer 与自主 resolver 的真实应用矩阵、普通 Win32/多进程真实应用的信息语义、长跑/升级/安装仍按各自证据保留，不从 unit、Hook 数量或新产品用语推断已完成 E01–E08。

IPv6 后置。当前信息容器不以 VM、驱动或存储隔离作为下一步门槛；候选硬件/设备信息也未因本次调整自动实现。
