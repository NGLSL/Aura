> 历史隔离路线：已由用户 2026-10-06 澄清后的[环境信息规格](../spec.md)取代。下方状态、依赖与完成要求只保留历史，不授权继续执行。

# Aura 轻量级环境容器总规格

Status: ready-for-agent
Date: 2026-10-04
Delivery scope: 完整轻量级 Container；V1 仅为首个交付阶段，不代表总目标完成
Implementation status: 已交付部分独立切片；完整 P0–P8 尚未完成，见 [实施证据](../evidence/implementation-progress.md)

## Problem Statement

用户希望反复使用多个环境运行同一 Windows 程序，保存各环境的配置和数据，同时共享必要的宿主资源。当前 Aura 已有 Application、Profile、EnvironmentSession、RuntimeInstance、Job 和 Runtime，但一次 Run 不是持久环境：关闭 GUI 后应用继续运行，重新打开却不能接管；Runtime 已加载也不足以证明实际 Profile、全部关键 Hook 和子进程覆盖符合当前展示。

当前还存在常见读取入口不一致、Packaged PID 复用、浏览器 renderer 部分覆盖等问题。直接增加文件/Registry Overlay 或 WFP，并不能消除这些控制面问题，也不能自动得到安全沙箱或完整网络命名空间。

本规格保存完整轻量级 Container 的目标。第一阶段解决持久身份、运行监管、准确保证；后续必须实现并验收 DNS、网络策略、文件/Registry 隔离、对象及代执行边界、驱动交付。V1 完成不能关闭整个 Container 目标。实施顺序、模块责任、阶段产物、风险和最终完成条件见[完整实施规划](2026-10-06-isolation-implementation-plan.md)。

## Solution

### V1 用户体验

用户创建命名的“环境工作区”，选择现有 Profile，在工作区内启动一个或多个普通 Win32 Application / Command。工作区身份与元数据长期保存；每次启动产生独立 RuntimeInstance，使用当时的不可变配置快照。

后台 Supervisor 管理这些运行实例。关闭 GUI 不终止应用，重新打开后可以查看和停止属于当前用户的实例。实例显示实际 Runtime/Profile 身份、注入时机、已验证能力及覆盖限制。能力无法满足请求时明确失败，不将部分覆盖显示为完整成功。

V1 的文件和 Registry 大部分仍采用现有 Compatibility 行为。工作区数据目录只保存 Aura 自身元数据、快照和审计；**创建这个目录不代表目标应用的 AppData、Temp 或 Registry 已被隔离**。UI 不提供尚未实现的“文件隔离已开启”“安全容器”或“完整 DNS 防泄漏”标识。

### 产品模式与保证

| 模式 | 本规格状态 | 承诺 |
| --- | --- | --- |
| Compatibility | V1 实现 | 持久工作区 + 受支持的 Environment View + 后台监管；目标保留宿主权限 |
| Container | 总目标，分阶段实施 | 通过验收的存储作用域和 Session 网络策略；必须列出未覆盖面 |
| Strong | 后续研究 | Restricted Token / AppContainer / 其他 backend 的权限隔离；不预先承诺兼容 |

Compatibility 仍是默认模式。V1 不接受 Container/Strong 配置后静默转为 Compatibility，应返回“不支持此模式”。

Environment View、存储写入保护、网络连接策略、数据读取权限是不同能力。只读宿主回退不代表保密性；共享 RW 目录明确允许修改宿主数据；严格 API DNS 路由不代表应用自带 DoH 被阻断。

## User Stories

1. 作为用户，我希望创建命名工作区并选择 Profile，以便长期重复使用同一环境设置。
2. 作为用户，我希望环境 A 和 B 有稳定且不同的身份，以便区分配置和运行记录。
3. 作为用户，我希望每次运行产生独立实例，以便同一工作区管理多个程序。
4. 作为用户，我希望修改工作区设置只影响之后的启动，以便不改变运行中的应用。
5. 作为用户，我希望 Profile 修改或删除有明确处理，以便不会用错误或缺失的配置启动。
6. 作为用户，我希望关闭 GUI 后程序继续运行，以便保持现有退出和升级行为。
7. 作为用户，我希望重新打开 GUI 后看到仍在运行的实例，以便继续管理和停止。
8. 作为用户，我希望停止实例只终止该实例的已纳管进程，以便不影响其他工作区或宿主程序。
9. 作为用户，我希望一次停止整个工作区的运行实例，以便清理当前活动。
10. 作为用户，我希望看到实际生效的 Profile 和 Runtime 身份，以便判断启动是否成功。
11. 作为用户，我希望部分覆盖有具体原因，以便不会把浏览器 renderer 或 PostActivation 当成完整覆盖。
12. 作为用户，我希望所需能力缺失时启动失败，以便不会静默使用宿主环境。
13. 作为用户，我希望同一已有 PID 不被两个不同 Profile 认领，以便避免假成功和互相覆盖。
14. 作为用户，我希望命令行和 GUI 使用相同规则，以便行为一致。
15. 作为用户，我希望失败和后台断连有可理解的状态，以便区分启动失败、应用退出和监管丢失。
16. 作为用户，我希望删除工作区前确认活动与数据范围，以便不会误删应用数据或宿主目录。
17. 作为用户，我希望已有 Application/Profile 和旧启动方式继续可用，以便升级无需重建配置。
18. 作为用户，我希望审计帮助定位覆盖缺口，同时不默认记录文件内容、凭据和完整 DNS 域名。
19. 作为用户，我希望清楚知道 V1 尚无 AppData/Registry Overlay，以便不把持久目录误认为数据隔离。
20. 作为维护者，我希望真实进程身份与能力可独立验收，以便后续驱动不会建立在错误归属之上。
21. 作为维护者，我希望后台崩溃和升级有明确行为，以便不会误杀宿主或错误恢复实例。
22. 作为维护者，我希望资源开销有实测报告，以便不用附件中的推测数字作为性能承诺。
23. 作为用户，我希望同一应用在 A/B 保存不同 AppData、Temp 与应用 Registry，以便长期使用独立应用状态。
24. 作为用户，我希望修改或删除受保护文件不修改宿主，以便保留宿主数据。
25. 作为用户，我希望显式选择共享只读或可写目录，以便使用源码、下载等必要资源。
26. 作为用户，我希望配置 UDP/TCP/DoT/DoH 与严格 DNS，以便上游失败不会回退宿主。
27. 作为用户，我希望环境连接策略独立生效，以便限制直接 IP 和未允许出口。
28. 作为用户，我希望了解任意 HTTPS、代执行与 IPC 边界，以便正确判断隔离能力。
29. 作为用户，我希望克隆、重置和导出具有可靠数据语义，以便操作不会损坏环境。
30. 作为用户，我希望驱动缺失时 Container 拒绝启动，以便不会静默降级。
31. 作为维护者，我希望各阶段依赖、证据与阻塞长期可查，以便以后恢复完整实施。

## Implementation Decisions

### 1. 领域对象和身份

- **Container** 是持久工作区聚合，至少包含 schema version、稳定 UUID、名称、默认 Profile 引用、模式和创建时间。名称不是安全身份，可以重复；操作使用 UUID。
- **EnvironmentSession** 保留为一次 Run 的控制面聚合，不重命名、不兼任持久 Container。**RuntimeInstance** 继续表示一次运行记录，新增可选 Container ID 和有效配置快照身份。
- **Profile** 保持现有环境配置职责。Container 引用现有 Profile，不复制另一套长期可编辑 Profile。启动前解析并验证，Supervisor 为每次 Run 保存完整、不可变的有效配置快照及版本/摘要。
- 更改 Container 默认 Profile 或原 Profile，只影响下一次 Run。Profile 被删除或当前配置无效时新启动失败；已有实例继续使用启动快照。
- Process Identity 至少包含 PID 和创建时间/等价 generation，避免 PID 复用造成认领、Stop、策略恢复错误。Profile ID、Instance ID 和 Runtime 实际身份分别验证，不用环境变量或同名 DLL 标记代替。

### 2. 启动范围与兼容

- V1 工作区启动只支持普通 Win32 Executable 和 Command，保留当前 x64/x86 能力范围。Packaged/AUMID、AppContainer、Protected Process、高于 Supervisor 能力的目标在工作区模式返回明确不支持。
- 既有直接 Application/Profile 启动不强制迁移为 Container，保留现有兼容入口；两种入口共享身份和能力验证，不能保留同 PID/不同 Profile 假成功。
- Windows Terminal 和其他 brokered console 启动在 V1 工作区入口不支持；不得悄悄改为 Direct。旧入口保持已有支持与已说明边界。
- 本版工作区要求子进程继承。Application 关闭继承时拒绝工作区启动，并指出可使用旧 root-only 入口；不自动改用户配置。
- 当前用户、相同或更低完整性级别的运行由同等权限 Supervisor 管理。不同用户或完整性级别的 Supervisor 是独立管理范围；普通 GUI 不静默请求提权，也不向任意高权限服务发送管理命令。

### 3. 后台 Supervisor 与控制通道

- 在现有 Broker/Session/Job 能力上增加独立 Supervisor，后台持有 Job、运行快照、进程对象和状态。GUI/CLI 是客户端；GUI 退出不销毁监督职责。
- V1 不安装 Windows 服务、不修改防火墙、不注册自启动。Supervisor 在首次需要时按需启动，使用后台/隐藏方式。
- 每用户、每完整性级别范围最多一个有效 Supervisor；实例 generation 和协议版本用于识别旧连接。重复启动/连接、命令幂等和重连必须有确定行为。
- 控制面与 Runtime bootstrap 的授权能力分开。认证实际连接者的 Windows 身份/进程，校验对象所有权；目标 Runtime 只能申请自身允许的配置/状态，不可提交任意 Profile、认领其他 PID或执行 Stop/Delete。远程访问拒绝，不能仅相信报文携带的 PID/Container ID。
- 持久化已纳管实例身份、配置摘要、Job 名称和必要恢复信息；重连先验证现存进程、Runtime 身份及可控制的 Job，再恢复为 Running。
- Supervisor 崩溃不自动终止全部应用。新 Supervisor 可以恢复仍有可靠身份和控制句柄的实例；无法证明或接管时显示 **TrackingLost**，不伪造 Running，不凭旧 PID 杀进程。该 Container 拒绝新 Run，直到现存实例退出或恢复可靠控制；其他身份及策略完整的 Container 不因一个实例失联而无条件禁用。
- OS 重启不自动重启目标。确认进程已经不存在后将旧运行记录结束，保留 Container；不得把 PID 恰好重用的宿主程序当作旧实例。
- 升级先握手检查运行时和协议兼容。不能在活动实例仍依赖旧 Runtime bundle 时删除该 bundle；无法安全接续时拒绝后台升级并说明原因。

### 4. 启动事务与失败行为

- 根进程启动顺序为：验证请求与有效快照 → 准备后台归属/引导 → 挂起创建并注入 → 加入对应 Job → 在经过验证的启动门控下完成 Runtime 实际身份与能力确认 → 发布实例并放行应用入口。
- Detours 的挂起创建只设置加载路径，不代表 Runtime 已执行；不能等待初始挂起线程不可能发送的 ACK。先验证受控 loader/bootstrap 启动与应用入口门控，在应用入口放行前完成确认，避免在 DllMain 的 loader lock 内阻塞等待管理命令。caller-requested suspended 组合单独验证，无法满足时明确拒绝，不能先运行用户代码再假称仍挂起。握手有截止时间；通知发送成功不等于后台已确认。
- Job 未允许 breakaway，常规子进程自然归组；Job 成员身份不能替代 Runtime 验证。子进程绑定也必须确认有效身份，不能 best-effort 通知后宣称完整传播。
- PID 已属于另一个 Instance/Profile 时拒绝认领；同 Profile 也不能把已有进程伪装成新实例。重复管理请求只能返回原实例的幂等结果。
- 配置、注入、Job、关键能力、身份确认或 Resume 失败，都清理本次新建资源并返回错误。只终止本次创建且拥有的进程，不终止已存在宿主进程。
- 修复 Detours 失败路径的句柄清理所有权，避免重复关闭；每个 Process/Thread/Job handle 只有一个明确 owner。
- Profile immutable，禁止用再次 LoadLibrary、改环境变量或重新绑定 PID 热更新运行中的环境。

### 5. 能力与用户展示

- 保留现有 IsolationGuarantee 作为**注入时机**指标，另建立实际能力与进程覆盖报告。FullPreExecution 不等于安全隔离，也不等于所有后续子进程已注入。
- 运行成功的必要能力包括根进程 Runtime/配置身份、必要进程传播入口和启动请求启用的严格策略。缺失时明确失败；不将“部分 Hook 成功”的计数当作能力确认。
- Coverage 状态区分 Verified、Partial、Unsupported、Unverified。Partial 必须包含可操作的原因和受影响能力；GUI/CLI 使用相同事实。
- Chromium sandboxed renderer 是已知不支持 Runtime 注入的路径，保留浏览器 sandbox，不循环尝试注入。不将该子进程或其 Environment View 标记 Verified；受支持的 browser-level policy 可以单独报告。
- WMI、COM、Shell 复用、系统服务和直接 Native 启动等未覆盖路径必须列在能力说明和验收矩阵。V1 不承诺拦住所有代执行，不把宿主服务全局注入/拦截作为补救。
- V1 不因浏览器存在部分覆盖就伪称完整进程树保证；后续请求完整树能力的模式必须拒绝无法满足的目标。
- UI 最小包括工作区列表/创建编辑、选择 Profile/启动、实例状态与能力、Stop/Stop all、后台重连状态。实际产品用语为“环境工作区”；不显示尚未实现的安全/存储隔离开关。

### 6. 存储、并发与删除

- 工作区存储使用项目现有 TOML 约定和独立 schema version；数据根位于 Aura 用户数据下，由 UUID派生，用户名称不参与路径生成。配置写入原子化并明确损坏/版本不支持的错误。
- V1 保存元数据、运行快照和审计；可创建预留的数据目录，但不重定向应用文件。现有用户配置、宿主 AppData/Temp 和 Registry 不因创建工作区而修改。
- 同一工作区允许多个不同实例并发运行，快照独立；它们当前仍共享宿主应用数据，这是 Compatibility 的明确行为。应用单实例机制可能将请求交给已有进程；未确认实际新实例时不记录新 Running。
- Stop 单实例只针对验证过归属的 Job/进程；Stop all 针对命令时的工作区实例集合并与新 Run串行，不能误停之后创建的实例或其他工作区。
- 删除分离“删除工作区登记”和“清除 Aura 管理数据”。存在活动实例或 TrackingLost 时拒绝删除/清空；不隐式 Stop。
- 数据清除需用户明确确认清单，并验证绝对路径在该 UUID 的管理根内；遇到 reparse point/外部目标拒绝递归清理。不得清除宿主共享目录、现有应用数据或旧 Profile。
- V1 不提供 clone/reset/export Overlay 功能，不声称元数据复制等于环境克隆。

### 7. DNS 与网络依赖

- 复用当前已完成的全 QTYPE DnsQuery 修复及支持的同步地址查询失败返回错误，不退回旧的非 A/AAAA Host 透传。
- DoH/DoT、typed ordered upstream、配置化 strict 的协议和迁移契约以现有 DNS transports 规格为准，不在这里重复定义。该独立规格未完成时，工作区只暴露当前已实现 DNS 能力，并明确未覆盖的输入/入口。
- 后续提供 `strict=true` 时，配置、引导、Hook、请求版本/选项和所有支持解析入口失败都不得切 Host。bootstrap、TLS 身份、总 deadline、取消及服务代执行分别验收。
- DNS strict、WebRTC Strict、网络 allowlist 是不同策略，不用同一个开关代替。不允许任意 HTTPS 后声称已阻止所有 DoH；不自动添加 Profile 未配置的公共 DNS 或明文 fallback。
- V1 无 WFP 驱动、完整 network namespace 或强制代理；应用直接网络访问沿用当前能力与明确边界。

## Testing Decisions

### 主要验收入口

以公共 CLI + 实际注入 Probe + 独立宿主控制进程作为主要行为缝；GUI 只验证同一状态和错误的显示，不重造独立业务路径。复用现有启动、DNS、Registry、挂起/Resume、Job 退出和实例状态测试。新增 Supervisor 重启/认证夹具属于必要验收，不以 mock 的加载成功替代实际身份。

只对本次能力增加必要覆盖；阶段门禁使用实际 Windows 证据。测试进程如果已被 Aura 注入，必须先建立独立宿主控制，记录实际 module path/配置身份，不使用旧 Runtime 或宿主值误判结果。

### V1 完成条件

| 编号 | 行为与验收 |
| --- | --- |
| A01 | 创建 A/B、重启 GUI 后 UUID及配置保持；删除/编辑 A 不改变 B 与旧 Application/Profile。损坏和未知 schema明确报错。 |
| A02 | A/B 使用不同 Profile启动同一受控 Win32 Probe；根进程和受支持子进程的 Runtime路径、实际Profile/Instance/Container身份与环境值分别匹配，宿主控制不变。 |
| A03 | 修改工作区/原Profile后旧实例继续原快照，新实例使用新快照；删除Profile使新Run失败而不改旧实例。 |
| A04 | GUI关闭/重开不终止目标；重新连接后可读取实际状态和Stop；仅同用户同管理范围可接管。 |
| A05 | 后台退出/崩溃/重启分别验证可恢复Running与TrackingLost；PID重用、失去Job、身份不匹配不得错误恢复或Stop；A失联不阻断独立B，A恢复必须重验身份与控制能力。 |
| A06 | 根注入/Job/必要Hook/身份ACK/Resume失败均返回错误且无本次未受控活进程；caller-requested suspended保持约定挂起语义或明确拒绝不支持组合。注入失败句柄清理不重复关闭。 |
| A07 | 同PID/不同Profile认领被拒；已有单实例目标不被记作新实例；任何冲突不终止宿主已有程序。 |
| A08 | CreateProcess A/W、AsUser、cmd/PowerShell中间进程与x64/x86组合分别报告权限阻止/已验证/未支持；WithToken/Shell/Native/WMI等矩阵列真实结果，不从单入口推断全覆盖。 |
| A09 | 真实Chromium目标/renderer报告Partial或Unsupported，不循环注入、不误标全树；若未完成真实浏览器验证，只能记录Unverified，不能用三份Probe模拟替代。 |
| A10 | 伪造PID/Instance/Profile、非owner客户端、Runtime提交管理命令和远程控制均拒绝；认证失败不改状态。 |
| A11 | Stop/Stop all/并发Run/重复命令有确定结果；其他工作区、相同exe宿主实例不受影响。 |
| A12 | 活动/TrackingLost拒绝Delete；明确清空只删除UUID管理根内数据，junction/reparse/外部目录夹具不被触及。 |
| A13 | Packaged/AppContainer/不支持ConsoleHost/inherit-off请求明确失败；现有兼容入口与旧配置仍可用，不静默改启动方式。 |
| A14 | GUI/CLI显示一致的身份、能力、Partial理由和后台状态；没有“Overlay已启用”或“完整DNS隔离”的错误承诺。 |
| A15 | 安装升级保持活动实例所需Runtime bundle与协议能力；不兼容更新明确拒绝接续。OS重启后旧记录不误认新PID。 |
| A16 | 当前DNS全QTYPE、Profile错误、TCP、取消和原生free回归通过；DoH/DoT未实现时不显示为可用。 |

自动测试成功不替代安装升级、真实浏览器、x86实际注入与长期运行证据。每个必需项标注 verified/unsupported/unverified；必需交付项 unverified 时不得宣称 V1 全部完成。权限不足的skip保留原OS错误，不算通过。

### 后续存储/网络门禁

- Registry：限定HKCU子树的宿主/A/B读写、删除标记、合并枚举、多值读取、Native读取、WOW64、通知、两个进程与重启对照。宿主hive真实内容必须独立检查。
- 文件：限定NTFS夹具的copy-up、mapped writes、SQLite/WAL类保存、rename/replace/delete、合并枚举、小buffer、hardlink/junction、跨边界句柄和crash/reopen。失败不得改宿主；不支持操作明确拒绝。
- WFP：同一exe的宿主/A/B连接分别允许/拒绝；进程创建到策略绑定前无未授权窗口；PID复用、后台崩溃、卸载/更新、直接IP、UDP/TCP及brokered traffic均有证据。允许HTTPS场景明确不能证明零应用DoH。
- 驱动：独立测试环境、签名/发布路径、HVCI/Secure Boot、Verifier、恢复/卸载与其他filter兼容；fixture通过不算整个卷验收。

### 性能报告

测量后台空闲、单实例、多实例、启动/重连/Stop耗时及长期变化，区分private working set、total working set和private commit。报告控制面与目标应用开销，包含环境和原始数据。不采用未经当前实现测量的“20–50MB”“秒建”“每DLL几MB”作为验收承诺，也不以低占用替代正确性。

## Out of Scope

本节是首阶段 V1 的排除项。文件/Registry Overlay、WFP、驱动交付及数据管理仍属于总目标，完整契约与 F01–F10 终验见[实施规划](2026-10-06-isolation-implementation-plan.md)。安全沙箱、整个 C: 透明虚拟化与任意程序兼容不属于本总目标。

- V1 不实现透明File/Registry Overlay、整个C:虚拟化、内核WFP/minifilter/Registry driver，也不安装系统服务。
- 不提供安全沙箱、恶意syscall防御、反检测、宿主身份保密性、隔离所有IPC/COM或阻止全部系统broker代执行。
- 不禁用浏览器sandbox、Secure Boot、HVCI或证书校验来获得兼容。
- 不把Job、模块加载标记、只读回退、ProxyOnly或WFP连接策略等同于完整容器保证。
- 不改变Windows全局Locale/Region/Timezone/DNS，不全局封禁共享服务，不影响同exe的宿主实例。
- 不默认迁移旧配置到更强权限模式；不自动启动目标程序、删除用户数据、发布或安装驱动。

## Further Notes

### 演进里程碑与依赖

以下是交付顺序；M3–M5 也属于总目标。详细契约及总体验收以[完整实施规划](2026-10-06-isolation-implementation-plan.md)为准。

| 里程碑 | 交付与门禁 | 是否属于V1 |
| --- | --- | --- |
| M0 实际保证修复 | Runtime身份/能力确认、PID冲突拒绝、句柄ownership、部分覆盖展示；为后续可信基础 | 是 |
| M1 持久工作区 | Container稳定身份、配置、快照、普通Win32启动、GUI/CLI管理 | 是 |
| M2 后台监管 | Supervisor授权、Job所有权、重连/Stop、崩溃与升级恢复；A01–A16通过 | 是 |
| M3 DNS与网络原型 | 既有DNS规格独立实施；Session网络策略单独评估，明确DoH/代执行边界 | 否，独立规格 |
| M4 数据隔离原型 | 指定Registry与AppData/Temp的宿主/A/B行为证明；定义作用域及不支持操作 | 否，原型是总目标的门禁 |
| M5 Container后端 | 原型通过后制定正式typed存储/共享策略、driver发布、生命周期与真实应用规格；逐步实现kernel filters | 否，属于总目标的必要交付 |
| M6 Strong后端 | RestrictedToken/AppContainer/其他backend独立验证；依据兼容与权限证据决定 | 否，研究 |

M0→M1/M2形成V1；M3/M4在可信身份基础上独立推进；M5依赖相关原型和driver门禁；原型通过后必须继续实现，不能以V1完成代替总目标；M6不绑定V1发版。启动/读取/Native/IPC/broker矩阵贯穿全部里程碑，不留到最终阶段补救。

本规格采用现有产品退出契约、普通Win32优先、无驱动V1和明确Partial的决定。后续存储作用域、应用支持矩阵和网络策略以关联实施规划为基线；原型发现需调整契约时记录决定，不由实现 Agent 静默扩大保证。

关联材料：[可行性研究](../../container-feasibility/research.md)、[DNS transports 规格](../../dns-transports/spec.md)、[领域术语](../../../docs/CONTEXT.md)、[现有隔离等级](../../../docs/isolation-tiers.md)。可行性研究是一手资料与风险证据；V1实施范围和完成条件以本规格为准。这里只发布规格，不生成实现tickets、分支或代码。
