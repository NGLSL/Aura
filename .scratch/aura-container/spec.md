# Aura 环境配置规格

Status: ready-for-human
Date: 2026-10-06
Scope revision: 用户明确澄清，容器是 Profile 环境信息视图；撤销此前将文件/Registry 写入隔离、内核网络策略和驱动交付作为总目标的扩展。
Implementation status: 当前声明的信息视图范围已实现并完成验证。已有 DNS 与用户态成果、四个可选身份字段、真实 Win32/Edge 对照及双架构恢复证据见[最终身份与 IPC 验收](evidence/profile-identity-and-ipc.md)；不延伸为任意读取入口或安全隔离保证。

## 目标

用户在 Aura 中创建环境配置（Environment Profile），在应用页为应用选择环境配置并启动，让受支持的程序及子进程读取指定的环境信息，减少通过已覆盖入口读取到宿主信息的情况。同一程序在 Host、A、B 下分别呈现 Host、Profile A、Profile B 的信息；宿主系统配置保持原样。

环境配置页只负责设置，不提供应用选择或启动入口；应用页负责选择应用及配置并启动，运行实例页负责实例管理。不要求另外新建或关联容器。GUI 自动管理后端运行作用域，复用原 Container/Supervisor 协议与旧数据，保留历史目录与 metadata。记录按 RunResult.profile_id 的运行时快照事实归属；后端批量停止精确限定所选 Profile、当前 generation 且可跟踪的实例，不能根据旧容器当前关联配置扩大范围。

每次运行使用不可变有效配置快照，运行中不热更新。目标程序继续使用宿主文件系统、GPU、网络和用户权限。文件写入、Registry 写入和出口访问权限隔离不属于本规格。

“防泄漏”必须写成具体能力和读取入口的保证，不能以容器名称或 DLL 已加载宣称所有宿主信息都不可获取。

## 术语与现有模型

- **Environment Profile**：定义支持的环境信息视图与 DNS/浏览器策略。
- **Container / 环境工作区**：保留为内部运行作用域和旧数据兼容模型，由 GUI 自动管理，不再是用户需要另行配置的产品对象。
- **RunSnapshot**：一次运行的有效配置快照，绑定 Container/Profile/Instance 身份。
- **RuntimeInstance**：一次运行及其受支持进程树。
- **Coverage**：按能力、读取入口和进程报告 Verified、Partial、Unsupported、Unverified。

现有代码的 Container 聚合与 Compatibility 模式可复用。序列化的 Container/Strong 枚举值不等于产品功能已启用，本次不得改成接受它们，也不强制迁移、重命名或删除用户配置。

## 信息能力范围

| 能力 | 当前 Profile / 入口 | 当前目标 |
| --- | --- | --- |
| Locale、Region、UI Language | Windows locale/geo/language 与 CRT 入口 | 与 Profile 一致；对应入口逐项验证 |
| Timezone | Windows/IANA ID、本地时间换算、受支持 WinRT 入口 | 虚拟时区、真实 UTC 时间线；不修改系统时钟 |
| 环境变量 | 独立 Environment Block 与已实现语言变量规则 | 按启动快照生效，说明程序自行修改的语义 |
| Registry 信息读视图 | 现有白名单内的地域、时区、DNS 等读入口 | 与其他视图一致；不扩展为完整 Registry Sandbox |
| DNS 配置视图与解析 | 受支持 Windows resolver API、任意 QTYPE | UDP/TCP/DoT/DoH；VirtualView strict 无 Host fallback |
| 浏览器/WebRTC 策略 | 已识别 engine 的 policy 与现有 Runtime guard | 报告政策和实际覆盖；不把 policy 当作全 renderer 保证 |
| Hostname、用户名、MAC、MachineGuid | 可选 IdentityProfile，指定 Win32 读入口 | 按[身份读视图规格](identity-spec.md)实现与验收；CPU/GPU/磁盘身份后置 |

不能通过设置 USERNAME、COMPUTERNAME 等环境变量，就宣称同名系统 API、WMI 或其他通道已虚拟化。候选字段不得生成无意义随机值或通过伪造检测结果取得“通过”。不实现反检测或隐藏 Aura。

## 实现契约

1. 复用 Profile → Storage → GUI/CLI → RunSnapshot → Launcher/Broker → Runtime → Probe 链路，不新增第二套 Profile 配置源。
2. Profile 修改只影响后续运行；配置缺失、无效或注入失败时明确拒绝启动。实际 Runtime、Profile 和配置身份需验证，不凭 DLL 名称或 ENV 自报判定成功。
3. 常规受支持子进程继承同一快照。PID 与创建时间用于归属及恢复；无法覆盖的特殊启动、提权、COM/Shell/服务代执行明确报告。
4. 保留现有用户态 Supervisor、Job、GUI 重连、Stop 和混合架构恢复。Job 用于管理，不提供安全边界；停止 A 不影响 B 或 Host。
5. 每新增或修改一组信息 Hook，同步扩展 Probe，验证 API 的 A/W、缓冲区、错误语义及相关等价读取入口。只有实际覆盖的入口可标 Verified。
6. 对已声明支持的 Profile 信息入口，读取错误不得静默返回宿主值并继续报告 Verified。具体能力在实现前确定返回错误、拒绝启动或降为 Partial 的规则；不把全部旧 Hook 一律改为 fail-closed。DNS strict 保持单独的明确无宿主回退契约。
7. 支持矩阵以真实程序、入口和进程为单位；FullPreExecution 只说明注入时机，不代表所有信息路径已覆盖。
8. 宿主 Locale、Region、Timezone、DNS、系统服务及防火墙保持不变。本目标不安装 LocalSystem 服务或驱动。

## DNS 与浏览器边界

[DNS 独立规格](../dns-transports/spec.md)继续有效：所有 QTYPE 共用 Profile query engine，只有配置的 upstream 按顺序尝试；不得隐式补入系统 DNS；DoH bootstrap 和 TLS 路径遵守无 Host DNS 要求。

应用自带 UDP/TCP DNS、DoH/DoT/DoQ 不一定调用受支持的 Windows resolver。用户态 Profile 不能因此承诺全部 DNS 流量不泄漏。对有正式设置或策略的应用，可独立评估配置其 resolver；无法控制的应用必须报告覆盖限制，不通过默认引入 WFP 或第三方沙箱扩大本目标。

Chromium sandbox renderer 等不能注入的进程保留原应用 sandbox，并显示 Partial/Unsupported。需要实际测量 JS/Intl/时区、browser-level DNS 和 WebRTC 行为，不能从主进程 Probe 推断全部 renderer。

Profile 不改变真实公网出口 IP，不是 VPN/代理，也不保证网络身份匿名。IPv6 连接专项按用户要求后置，不阻塞当前 IPv4 transport；AAAA QTYPE 仍属于当前支持范围。

## 验收

| 编号 | 必须提供的证据 |
| --- | --- |
| E01 | 同一 Probe 在未注入 Host、A、B 下实际读取各自预期信息；每能力保存配置和原始输出 |
| E02 | Locale/Region/Language/Timezone/Env 与白名单读视图的一致性逐项通过；真实 UTC 时间线不改变 |
| E03 | 启动实际身份、不可变快照、受支持子进程和 x64/x86 路径；特殊入口明确列出限制 |
| E04 | 四种 DNS transport、任意 QTYPE、上游失败与 strict 无 Host fallback；证据区分配置视图、API 路由、应用 resolver |
| E05 | GUI 关闭/重连、Supervisor 重启恢复、Stop A 不影响 B/Host；TrackingLost 不伪造 Running |
| E06 | 一个普通 Win32 应用与一个多进程应用的实际读取矩阵；浏览器作为独立评估，不预先保证其 renderer 合格 |
| E07 | GUI/CLI 准确显示当前字段和覆盖；候选信息未实现时不显示已保护或完整匿名 |
| E08 | 未注入宿主对照确认系统配置不变；静态检查、自动化、实际注入、真实应用与安装结果分别记录 |

本规格验收不依赖 WFP/minifilter、文件 COW、Registry hive 隔离、驱动签名、Verifier 或虚拟机。无需 VM 不等于无需测试；真实注入和应用矩阵仍必须完成。

## 旧路线与既有成果

旧 P0–P8 隔离路线已被用户本次澄清取代，不再是本目标的完成条件：[旧总规格](history/2026-10-06-isolation-spec.md)、[旧规划](history/2026-10-06-isolation-implementation-plan.md)、[旧票据地图](history/2026-10-06-isolation-map.md)。归档中的相对链接已调整，旧文档只作历史证据。

复用已有 Profile、快照、用户态监管、DNS、Runtime 和恢复成果。保留可信服务与内核原型源码及已有证据，不安装、启用、删除或回滚；其未部署状态不阻塞环境信息容器验收。是否清理实验代码另立有明确范围的任务。

新执行顺序见 [实施规划](implementation-plan.md)，旧票如何复用见 [地图](map.md)。身份字段的实现与兼容验收见 [身份读视图规格](identity-spec.md)，运行结果按实际读取入口记录。
