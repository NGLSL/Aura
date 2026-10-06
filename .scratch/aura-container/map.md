# 环境信息容器执行地图

Status: ready-for-human
Date: 2026-10-06
Scope: 当前用户澄清的信息Profile目标；旧49票隔离路线不再作为整体执行计划。

[当前规格](spec.md) · [当前规划](implementation-plan.md) · [历史地图](history/2026-10-06-isolation-map.md)

## 当前 frontier

R1–R4 已按当前信息视图范围盘点与对齐。R6 增加用户确认的四个身份字段，R7 按职责拆分 IPC；最终读取、真实应用和恢复证据见[身份与 IPC 验收](evidence/profile-identity-and-ipc.md)。旧内核/存储路线继续保留为历史。

2026-10-06 已完成源码读取盘点、GUI/CLI 产品范围对齐、历史 metadata 保存/运行解耦及实际 Runtime 安装事实展示，见[代码对齐证据](evidence/environment-view-alignment.md)。这不自动完成真实应用信息语义矩阵；后续优先补声明支持的实际读取证据，不返回旧驱动路线。

| 工作包 | 关联旧票/证据 | 当前决定 |
| --- | --- | --- |
| R1 信息读取盘点 | 02–05、现有Profile/Runtime/Probe | 已盘点；区分字段支持、API覆盖、真实应用覆盖与候选信息 |
| R2 已有信息一致性 | 01–08、现有白名单读视图 | 复用已实现成果；按差异补缺口 |
| R3 DNS/浏览器 | 14–20、独立DNS规格 | 保留已有实现；无Host fallback仅按受支持路径声明 |
| R4 产品展示 | 08与现有GUI/CLI | 纠正安全/存储含义，配置兼容性保留 |
| R5 最终矩阵 | 09–13恢复证据、真实应用 | 声明入口验收已完成；安装/升级、长跑及任意读取路径不在证明范围 |
| R6 身份读视图 | identity-spec.md、双架构 Probe、hostname.exe | 主机名/用户名/MAC/MachineGuid；CPU/GPU/磁盘延期 |
| R7 IPC 职责拆分 | Launcher ipc.rs 与 ipc/ | 稳定入口，协议/Profile/会话/测试独立；保留线协议与状态机 |

## 旧票的执行规则

旧01–20中与上述信息容器目标一致的内容继续作为复用材料，不能把旧状态直接当作最新验收结论。旧21–49脱离当前执行frontier；旧票的状态只保留历史，不授权继续自研隔离后端。

旧文件未删除，入口在[历史地图](history/2026-10-06-isolation-map.md)。以后若用户重新明确要求存储/权限隔离，再独立评估范围和验收，不默认恢复旧P0–P8。历史map的相对票据链接已调整回原issues目录。

## 当前完成条件

[当前规格E01–E08](spec.md#验收)全部按声明支持范围验收。IPv6仍延期；文件隔离、Registry写隔离、WFP、驱动签名/加载/Verifier、Strong模式不是当前完成条件。
