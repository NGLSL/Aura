# Aura 轻量级容器实施票据地图

Status: ready-for-agent
Date: 2026-10-04
Delivery status: 已实施兼容工作区、Supervisor 与严格 DNS 的独立切片；整体仍未完成。当前验收与阻塞见 [实施证据](evidence/implementation-progress.md)。

[总规格](spec.md) · [完整实施规划](implementation-plan.md) · [研究依据](../container-feasibility/research.md) · [DNS 契约](../dns-transports/spec.md)

## 目标与执行规则

P0–P8 全部纳入，共 49 张独立票。01–13 完成只能代表第一阶段；49 通过最终验收后才能宣称完整目标完成。票的 ready-for-agent 表示已明确可领取的工作范围，不表示所有依赖或外部环境已就绪。Blocked by 均通过后才进入执行 frontier；原型门禁要求明确通过证据，失败报告不等于解除依赖。

每票限定负责模块，实现者保留无关改动，不同票存在共享模块时协调所有权，不以没有依赖边推断可以同时修改。CLI/实际注入 Probe/未注入宿主控制是主要验收路径；只有包含用户配置或显示变化的票扩展 GUI，不强迫每个内核实验写产品 UI。

驱动实验限制在明确准备的可恢复隔离环境。21 形成工具链/签名/环境报告，不要求 Agent 自动取得外部凭据；43 准备正式签名交付，缺少证书/altitude/平台批准时记录阻塞，不能自签替代正式验收。安装到宿主、对外提交、发布和清除用户数据仍需要覆盖实际操作的授权。拆票本身不执行这些操作。

已存在的 DNS 修复应先核对当前源码及实际 DLL 证据，不以票号重新实现或预先标完成。执行后保存实际验证结果，分别标源代码、自动化、实际注入、安装及长期运行证据。未支持路径必须明确拒绝或列入支持边界，声明支持项不得用 Unverified 过关。

## 全部工作票与阻塞关系

| 票 | 阶段 | 交付 | Blocked by |
| --- | --- | --- | --- |
| [01](issues/01-ticket.md) | P0 | 启动失败句柄和资源清理 | 无 |
| [02](issues/02-ticket.md) | P0 | 真实 Runtime 身份与授权握手 | [01](issues/01-ticket.md) |
| [03](issues/03-ticket.md) | P0 | 受控 bootstrap 与应用入口门控 | [02](issues/02-ticket.md) |
| [04](issues/04-ticket.md) | P0 | 子进程传播与身份确认 | [03](issues/03-ticket.md) |
| [05](issues/05-ticket.md) | P0 | 进程入口和真实应用覆盖矩阵 | [04](issues/04-ticket.md) |
| [06](issues/06-ticket.md) | P1 | 持久 Container 创建编辑与版本化保存 | 无 |
| [07](issues/07-ticket.md) | P1 | 不可变 Run 快照与 Profile 变更处理 | [06](issues/06-ticket.md), [02](issues/02-ticket.md) |
| [08](issues/08-ticket.md) | P1 | 工作区运行和 GUI/CLI 能力展示 | [07](issues/07-ticket.md), [03](issues/03-ticket.md), [05](issues/05-ticket.md) |
| [09](issues/09-ticket.md) | P2 | 独立 Supervisor 启动与认证控制 | [02](issues/02-ticket.md), [07](issues/07-ticket.md) |
| [10](issues/10-ticket.md) | P2 | Supervisor 持有 Job 与启动事务 | [09](issues/09-ticket.md), [03](issues/03-ticket.md) |
| [11](issues/11-ticket.md) | P2 | GUI 重连与幂等 Stop/Stop all | [10](issues/10-ticket.md) |
| [12](issues/12-ticket.md) | P2 | 崩溃恢复与 TrackingLost | [11](issues/11-ticket.md) |
| [13](issues/13-ticket.md) | P2 | 活动 Runtime bundle 与协议升级 | [12](issues/12-ticket.md), [08](issues/08-ticket.md) |
| [14](issues/14-ticket.md) | P3 | 有序 typed DNS 配置全链及迁移 | [07](issues/07-ticket.md) |
| [15](issues/15-ticket.md) | P3 | 任意 QTYPE Query Engine 与 UDP/TCP | [14](issues/14-ticket.md) |
| [16](issues/16-ticket.md) | P3 | 严格解析入口与异步取消 | [15](issues/15-ticket.md), [04](issues/04-ticket.md) |
| [17](issues/17-ticket.md) | P3 | DoT 传输与 TLS 校验 | [15](issues/15-ticket.md) |
| [18](issues/18-ticket.md) | P3 | DoH 无 Host DNS bootstrap 选型原型 | [14](issues/14-ticket.md) |
| [19](issues/19-ticket.md) | P3 | DoH 正式 transport | [18](issues/18-ticket.md), [15](issues/15-ticket.md) |
| [20](issues/20-ticket.md) | P3 | 四种 DNS 实际注入验收 | [16](issues/16-ticket.md), [17](issues/17-ticket.md), [19](issues/19-ticket.md), [11](issues/11-ticket.md) |
| [21](issues/21-ticket.md) | P4 | 驱动工具链、签名和实验环境预检 | 无 |
| [22](issues/22-ticket.md) | P4 | 内核进程归属与可信策略通道 | [21](issues/21-ticket.md), [02](issues/02-ticket.md), [03](issues/03-ticket.md), [10](issues/10-ticket.md) |
| [23](issues/23-ticket.md) | P4 | WFP 按进程 Deny/Host 原型 | [22](issues/22-ticket.md) |
| [24](issues/24-ticket.md) | P4 | Allowlist 与动态 DNS 策略原型 | [23](issues/23-ticket.md), [14](issues/14-ticket.md) |
| [25](issues/25-ticket.md) | P4 | 网络创建窗口、故障和代执行门禁 | [24](issues/24-ticket.md), [12](issues/12-ticket.md) |
| [26](issues/26-ticket.md) | P5 | 存储作用域与共享规则配置 | [06](issues/06-ticket.md) |
| [27](issues/27-ticket.md) | P5 | NTFS 可写打开 COW 原型 | [26](issues/26-ticket.md), [22](issues/22-ticket.md) |
| [28](issues/28-ticket.md) | P5 | 删除、rename、replace 与合并枚举 | [27](issues/27-ticket.md) |
| [29](issues/29-ticket.md) | P5 | 映射写、paging、WAL 和崩溃原型 | [27](issues/27-ticket.md) |
| [30](issues/30-ticket.md) | P5 | 别名、跨边界句柄与 ACL 矩阵 | [27](issues/27-ticket.md), [29](issues/29-ticket.md) |
| [31](issues/31-ticket.md) | P5 | HKCU 私有 union 读写删除 | [26](issues/26-ticket.md), [22](issues/22-ticket.md) |
| [32](issues/32-ticket.md) | P5 | Registry 枚举、通知、Native/WOW64 | [31](issues/31-ticket.md) |
| [33](issues/33-ticket.md) | P5 | 存储原型门禁与 backend 决定 | [28](issues/28-ticket.md), [29](issues/29-ticket.md), [30](issues/30-ticket.md), [32](issues/32-ticket.md) |
| [34](issues/34-ticket.md) | P6 | 正式 AppData/Temp 与共享目录 backend | [33](issues/33-ticket.md), [10](issues/10-ticket.md) |
| [35](issues/35-ticket.md) | P6 | 正式应用 Registry backend | [33](issues/33-ticket.md), [10](issues/10-ticket.md) |
| [36](issues/36-ticket.md) | P6 | 正式网络策略与 DNS 出口集成 | [25](issues/25-ticket.md), [20](issues/20-ticket.md), [10](issues/10-ticket.md) |
| [37](issues/37-ticket.md) | P6 | 命名对象、单实例与 broker 边界 | [05](issues/05-ticket.md), [22](issues/22-ticket.md) |
| [38](issues/38-ticket.md) | P6 | Container 启动事务与故障不降级 | [34](issues/34-ticket.md), [35](issues/35-ticket.md), [36](issues/36-ticket.md), [37](issues/37-ticket.md), [12](issues/12-ticket.md) |
| [39](issues/39-ticket.md) | P6 | Container GUI/CLI 与支持矩阵 | [38](issues/38-ticket.md), [08](issues/08-ticket.md) |
| [40](issues/40-ticket.md) | P7 | 停机克隆与新 UUID | [39](issues/39-ticket.md) |
| [41](issues/41-ticket.md) | P7 | 安全重置、删除与活动资源释放 | [39](issues/39-ticket.md) |
| [42](issues/42-ticket.md) | P7 | 停机导出导入与版本校验 | [40](issues/40-ticket.md), [41](issues/41-ticket.md) |
| [43](issues/43-ticket.md) | P7 | 正式签名与发布交付准备 | [21](issues/21-ticket.md), [38](issues/38-ticket.md) |
| [44](issues/44-ticket.md) | P7 | 安装升级卸载与回滚包 | [43](issues/43-ticket.md), [13](issues/13-ticket.md), [41](issues/41-ticket.md) |
| [45](issues/45-ticket.md) | P7 | 审计及数据管理产品界面 | [39](issues/39-ticket.md), [40](issues/40-ticket.md), [41](issues/41-ticket.md), [42](issues/42-ticket.md), [44](issues/44-ticket.md) |
| [46](issues/46-ticket.md) | P8 | Host/A/B 真实应用完整矩阵 | [45](issues/45-ticket.md) |
| [47](issues/47-ticket.md) | P8 | 故障、Verifier、HVCI 与 filter interop | [44](issues/44-ticket.md), [38](issues/38-ticket.md) |
| [48](issues/48-ticket.md) | P8 | 长跑与资源性能报告 | [46](issues/46-ticket.md), [47](issues/47-ticket.md) |
| [49](issues/49-ticket.md) | P8 | F01–F10 最终验收与保证文档 | [20](issues/20-ticket.md), [25](issues/25-ticket.md), [33](issues/33-ticket.md), [42](issues/42-ticket.md), [45](issues/45-ticket.md), [46](issues/46-ticket.md), [47](issues/47-ticket.md), [48](issues/48-ticket.md) |

## 可开始的 frontier 与阶段门槛

初始无前置票：01 启动失败清理、06 持久工作区 CRUD、21 驱动只读预检。21 只完成报告不自动允许 22 加载驱动；具体实验环境必须满足门槛。之后 26 可随 06 完成开始，不需要等待 DNS 或网络实验。

P3 的 DoT 与 DoH 在共享 Query Engine/config 契约完成后可独立推进。WFP 与存储原型在内核身份通道完成后可按模块分工；正式 Container 必须合并经过验证的实现，不能把 P4/P5 fixture 当 P6 成品。

| 门槛 | 必需票 | 失败时行为 |
| --- | --- | --- |
| 应用入口放行 | 03 | 不伪称初始 Resume 前 Runtime 已握手；挂起组合不能证明则明确拒绝 |
| 可恢复驱动实验 | 21、22 | 外部材料或环境缺失时阻塞驱动实验，继续无驱动独立票 |
| 无宿主 DNS 的 DoH 选型 | 18 | 不实施有 Host bootstrap 的 19；记录替代选型及重新验证 |
| 网络策略正确性 | 25 | 不启用正式 36，不全局封禁共享服务 |
| 存储对象与一致性 | 33 | 不启用正式 34/35，不以 Host 写回退隐藏原型失败 |
| 完整 Container 启动 | 38 | 任一必需 backend 缺失拒绝，不转 Compatibility |
| 正式驱动交付 | 43、44 | 不以测试签名、构建成功代替正式安装资格 |
| 最终完成 | 49 | F01–F10 任一必需项无证据则整体未完成 |

## 最终验收覆盖索引

| 总验收 | 主要执行票 |
| --- | --- |
| F01 身份、持久状态与监管 | 02–13、34–39、46 |
| F02 文件与 Registry 无未授权宿主写入 | 26–35、38、46–47 |
| F03 四种 DNS 与 strict 无 Host fallback | 14–20、36、46 |
| F04 按实例网络策略与生命周期 | 22–25、36、38、46–47 |
| F05 对象、单实例和代执行边界 | 05、25、37、39、46 |
| F06 崩溃、PID 重用和升级不降级 | 01–04、10–13、22–25、33、38、44、47 |
| F07 克隆、数据操作与恢复 | 40–42、45–47 |
| F08 驱动安装、签名和兼容 | 21–22、43–44、47 |
| F09 实际应用支持矩阵 | 05、37、39、46 |
| F10 正确展示、文档与资源实测 | 08、39、45、48–49 |

49 汇总上述证据且独立核对实际产物，不凭其他票的完成声明关闭总目标。不自动关闭或修改父规格状态。
