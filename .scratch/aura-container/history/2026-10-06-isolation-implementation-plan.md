> 历史隔离路线：已由用户 2026-10-06 澄清后的[环境信息规格](../spec.md)取代。下方状态、依赖与完成要求只保留历史，不授权继续执行。

# Aura 轻量级环境容器完整实施规划

Status: ready-for-agent
Date: 2026-10-04
Implementation status: 用户态基础与 IPv4 DNS 已实现并分项验证；完整容器后端仍在后续阶段，见 [当前进度](../evidence/implementation-progress.md)
Parent: [总规格](2026-10-06-isolation-spec.md)

## 目标与完成定义

2026-10-06 用户调整排期：IPv6 单列为后续专项，当前不实施、不阻塞本阶段 IPv4 DNS 或其他独立能力的完成。以下 IPv6 要求保留为延期阶段的契约；当前 F03/F04 按声明的 IPv4 支持范围验收，不因 IPv6 尚未执行判定该范围未完成。未来启用 IPv6 时补地址/路由、四传输、bootstrap、双架构和无 Host fallback 验证；AAAA 查询支持仍属于当前任意 QTYPE 能力。完整容器的 WFP、文件/Registry 后端和驱动资格等其他必需项不因这次延期被取消。

长期目标是共享宿主 Windows 内核、按持久 Container 保存配置和应用状态的轻量环境容器。普通 Win32 应用可在不同环境运行；受保护作用域内的写入、删除及 Registry 状态彼此隔离；共享目录和出口由明确策略控制；GUI 退出后后台继续监管。目标不是实现独立 Windows、虚拟 GPU 或完整网络命名空间。

用户要求保存完整实现路线。以下 P0–P8 均属于总目标；原型是验证路径的中间产物，不能作为最终容器替代品。单独完成持久目录、Supervisor 或用户态 Hook 不得将本规划标记完成。若平台或兼容性门槛无法满足，保留未完成状态、证据和替代路径；不得悄悄缩为 V1。

Container 模式的“完整”指本文件定义的能力闭环及公开支持矩阵，不指任意 Windows 程序、整个 C:、所有系统服务或所有 IPC 的透明隔离。读宿主回退不保护宿主机密；没有安全权限边界的 Container 不对恶意软件提供逃逸防护。Strong 权限 backend 单独研究，不作为本目标的隐含承诺。

## 已有证据与仍待证明

当前代码具备 Profile、一次 Run 的 EnvironmentSession、RuntimeInstance、Job、注入及 Broker。当前 DNS 修改覆盖全 QTYPE 与部分同步解析入口；这不等于 DoH/DoT 或内核出口约束完成。已知 renderer 跳过、WMI 代执行、实际身份握手缺失以及 Registry 读取入口不一致，均应进入支持矩阵。

[可行性研究](../../container-feasibility/research.md)已核查微软关于 minifilter、Registry callback、WFP、AppContainer、签名及 HVCI 的资料，并区分平台能力与工程推断。尚未证明透明 union、mapped writes、跨边界句柄、代执行归属和正式驱动交付；未测量所谓 20–50 MB 开销。后续实施前刷新易变化的驱动政策和工具链要求。

## 目标架构与职责

| 层 | 责任 | 不承担的保证 |
| --- | --- | --- |
| GUI / CLI | 创建、配置、运行、停止、导出、查看实际能力 | 不自行决定进程归属，不直接操作驱动策略 |
| Rust 领域与持久化 | Container、不可变 Run 快照、版本与迁移、存储/网络策略 | 不复制现有 Profile 形成第二套环境配置 |
| Supervisor | 启动事务、Job、真实身份、策略绑定、恢复、审计 | Job 成员不等于全部入口注入成功 |
| Runtime | 受支持的 Environment View、API DNS、进程传播与能力报告 | 不用用户态 Hook 宣称拦住全部 Native/服务路径 |
| DNS Query Engine | 任意 QTYPE、UDP/TCP/DoT/DoH、严格失败与取消 | 不自动阻断应用自己的加密 DNS |
| 文件 backend | 指定 NTFS 作用域的 lazy host view、COW、whiteout、合并枚举 | 不自动覆盖其他卷、网络盘和整个系统盘 |
| Registry backend | 指定应用子树的私有状态、host fallback、删除与枚举语义 | 不整体替换 HKLM 破坏系统配置 |
| 网络 backend | 按真实进程归属约束连接，管理生命周期与拒绝策略 | WFP 不等于独立网络 namespace |
| 驱动与安装组件 | 内核归属、必要文件/Registry/WFP 拦截、签名部署、恢复 | 不关闭 Secure Boot、HVCI 或目标应用 sandbox |

Driver 划分在原型后决定；不预设一个巨型驱动，也不因模块拆分复制身份系统。控制面用 Rust，现有 C++ Runtime 继续复用；内核语言和 WDK 工程按已验证的 API、构建和交付条件选择，不能因为 Rust 工具链升级就默认内核实现可用。

## 最终产品契约

### 配置与启动

Container 保存稳定 UUID、Profile 引用、模式、存储规则、网络规则、对象策略及 schema。每次 Run 冻结完整有效配置。Compatibility 默认保留旧行为；Container 是显式模式，默认 strict DNS，任一必需 backend 不可用则拒绝启动，不能回退宿主或 Compatibility。

容器需要驱动时由经授权安装的系统组件提供能力；GUI 不隐式提权。持久数据与驱动管理分离，每用户 Supervisor 不能控制其他用户工作区。内核只接受可信控制组件下发的版本化策略，目标 Runtime 无权自行认领 PID 或更换策略。

启动事务必须在应用代码执行前完成身份、必要策略和能力门控。子进程归属在其可执行受保护 I/O/连接前建立；无法纳管的路径拒绝或明确标为未支持，不显示全树覆盖。Supervisor 崩溃后，已绑定内核策略继续执行；用户态必需组件消失时冻结/拒绝受保护操作或终止已确认归属的目标，具体按原型选定可验证策略，绝不切回宿主写入。Compatibility 保留总规格中不自动终止的退出契约。

### 文件视图

首个正式存储范围是支持矩阵内应用的本用户 AppData、LocalAppData、Temp 与显式加入的本地 NTFS 数据目录，不覆盖整个 C:。系统程序和运行库保留宿主只读视图；可写路径必须有明确规则，不能将未匹配写入自动放行。

规则是隔离写入、共享只读、共享可写或拒绝。采用规范化对象与最终路径授权，冲突采用最具体有效规则，等价路径必须得到同一结果；无法确定归属时拒绝受保护访问。显式共享可写是用户允许修改宿主的例外，GUI 必须可见。驱动和 overlay 管理根不能被目标绕过规则直接访问。

读取先查私有 overlay，再查未被 tombstone 遮蔽的 host。第一次可写打开/映射前形成私有 backing object，后续 paging I/O 基于已绑定对象处理，不能临时按当前线程 PID 路由。目录创建、删除、rename、replace、合并枚举与通知必须一致；失败或崩溃不能使已删除 host 项再次出现。

lazy host view 不是冻结镜像：未 copy-up 项可能随宿主变化。首版明确展示此语义，不把导出称为完整宿主快照。hardlink、ADS、open-by-ID、reparse、跨卷 rename、网络卷等逐项记录支持或拒绝；绕过到未支持路径不能写宿主。继承或 DuplicateHandle 得到的跨边界可写句柄必须拒绝或提供已证明的隔离行为；直接路径测试通过不能代替句柄验证。

### Registry 视图

首个正式范围是支持矩阵中明确列出的应用 HKCU 子树；必要 HKLM 应用子树单独配置和验证，不整体重映射系统根。私有 hive/存储后端不等于 union 实现。

支持 key/value 创建、读取、修改、删除、key/value tombstone、合并枚举与数量信息、WOW64 视图、通知和重启持久化。Win32 与 Native 入口必须在声明作用域保持一致；Registry handle/object 绑定归属，不仅按调用时 PID 判定。与文件一样，未匹配写入不能静默落宿主；共享例外必须显式。ACL、事务、符号链接、COM 注册等未支持能力逐项拒绝或列明限制，不把局部 fixture 成功扩大为完整 Registry。

### DNS 与网络

DNS 细节复用[DNS transports 规格](../../dns-transports/spec.md)：所有 QTYPE 共用传输抽象、用户配置顺序、UDP/TCP/DoT/DoH、显式 bootstrap、TLS 校验、总超时、取消、strict no-host-fallback。允许明文 fallback 必须由配置明确决定；不补入公共 DNS。

网络至少提供 Host、Deny、Allowlist 策略；Container 明确选择出口配置，不继承未知宽松规则。允许项包括目的地址/端口/协议及生命周期，DNS 动态地址绑定必须定义 TTL、更新与旧连接处理。Loopback、IPv4/IPv6、UDP/QUIC、直接 IP、监听/入站单独配置和测试。

直接 53/853 等只允许配置上游或受控 DNS 路径。允许任意 HTTPS 的策略明确不能保证阻止应用自带 DoH；不能以检测已知 DoH IP 列表替代保证。需要更强出口时使用实际目的地约束或可验证受控代理策略。服务代执行不能按服务 PID 误归属某一 Container；不能全局阻断共享服务。无法归属的代执行是严格模式的兼容阻塞项，不能记为已隔离。

### 对象、单实例与数据操作

纳入 named mutex/event、共享内存、named pipe 及单实例转交测试。只有经过证明的对象范围可按 Container 命名或权限区分，不能任意重写系统 IPC。ALPC/COM/Shell/WMI/服务、Packaged 和高完整性目标分别建立边界；需要完整隔离且无法覆盖的应用不进入 Container 支持名单。保持原应用 sandbox。

多实例同一 Container 共享该 Container 的应用数据，不默认提供每 Run 独立数据。克隆生成新 UUID，复制已停止环境的私有状态及策略；共享目录保持引用并明确提示。重置/删除必须先停止并释放对象、映射、Registry 与策略引用。首版克隆/导出只支持停机一致性，导出只含私有数据和清单，不隐含复制宿主或共享目录。导入验证版本、路径、大小、ACL及后端能力；失败不覆盖已有环境。

## 分阶段工作包与依赖

| 阶段 | 负责模块与交付 | 验收门槛及后续 |
| --- | --- | --- |
| P0 可信基础 | Launcher/Runtime/Broker：实际身份、入口门控、Hook 能力、句柄 ownership、PID 冲突、入口矩阵 | 总规格身份/失败项通过；门控原型不能实现则记录替代方案，不能假称 PreExecution |
| P1 持久工作区 | Core/Storage/GUI/CLI：Container、快照、schema、实例关联、可见能力 | A/B、配置升级及不可变快照；不宣称数据已隔离 |
| P2 后台监管 | Supervisor/IPC/Job：授权、后台寿命、重连、Stop、TrackingLost、版本兼容 | 总规格 A01–A16；单个失联 A 不阻断独立 B；旧 Runtime bundle 保留 |
| P3 完整 DNS | 配置/Runtime/Probe：全部 transports 与严格解析入口 | DNS 独立规格全部验收；无宿主 bootstrap/fallback；GUI 退出持续有效 |
| P4 内核准备和身份/网络原型 | WDK/可信控制通道/进程归属/WFP：最小可恢复驱动，支持环境预检 | 同 exe 的 host/A/B、PID 复用、创建窗口、直接 IP、组件消失；签名路径与测试 VM 可用 |
| P5 存储语义原型 | 文件/Registry backend：有限 NTFS 与 HKCU fixture 的完整行为 | mapped writes、句柄传递、删除/枚举、WAL、WOW64、Native、崩溃重开；host/A/B 独立证据 |
| P6 Container 集成 | 将 P4/P5 已证明机制下沉并接入不可变策略、对象作用域、生命周期 | 正式 AppData/Temp/应用 Registry、共享规则、运行失败隔离；真实应用支持矩阵 |
| P7 数据管理与产品交付 | clone/reset/export/import、安装/升级/卸载、GUI 能力与审计 | 停机一致性、活动资源拒绝、签名/HVCI/恢复/升级完整验证 |
| P8 最终资格验证 | 实际安装、长跑、真实应用及 filter interop、性能报告、文档 | 下列 F01–F10 全部通过后才能关闭完整 Container 目标 |

P0→P1/P2；P3 可在可信配置契约后独立推进；P4/P5 在身份设计一致时可并行研究，最终文件/Registry 驱动能力须在 P6 证明；P6 依赖 P2/P3/P4/P5，P7/P8 不可省略。先 x64，x86 随各用户态阶段建立真实证据；内核 backend 不以 x86 Runtime 存在替代 WOW64 证明。Strong/AppContainer 权限 backend 不阻塞已定义的环境容器，但不能借此宣传安全沙箱。

## 最终验收

| 编号 | 完成条件 |
| --- | --- |
| F01 | 同应用 host/A/B 的配置、Runtime/Container/Instance 真实身份、实际文件和 Registry 各自符合契约，关闭 GUI 与重启后台可继续管理 |
| F02 | 支持作用域的 Win32/Native/映射/句柄/枚举/删除/原子保存全路径没有未授权宿主写入；host 独立观察而非注入进程自证 |
| F03 | 四种 DNS transports、任意 QTYPE、TLS/bootstrap/失败/取消全部通过；strict 无 Host 回退，实际网络证据包含负向案例 |
| F04 | host/A/B 同 exe 的网络规则互不影响；无策略绑定前连接窗口；IPv6/QUIC/直接 IP/loopback/服务边界符合声明 |
| F05 | 单实例及命名对象不会把 A 请求默默交给 host/B 执行；未支持 broker/IPC 路径拒绝或使应用明确不受支持 |
| F06 | Supervisor/驱动/Runtime 故障、PID 重用及组件升级均无错误认领、宿主误杀或保护静默降级 |
| F07 | 停机 clone/reset/export/import、迁移及损坏恢复不影响其他 Container/共享目录/宿主，活动映射与 hive 不被提前删除 |
| F08 | 正式支持系统的安装、驱动签名、Secure Boot/HVCI、卸载、失败恢复与 filter interop有实机/隔离环境证据；安装器失败可回滚 |
| F09 | 支持矩阵至少覆盖受控 Probe、一个普通 Win32 GUI 应用、一个多进程应用、SQLite/WAL 工作负载及实际浏览器评估；不预先保证浏览器合格，失败须列为不支持 |
| F10 | GUI/CLI/文档准确展示作用域、共享例外和边界；启动/空闲/多实例/长跑资源实测归档；全部必需项无未验证状态 |

Probe 和 fixture 是主要自动化缝，不能替代实际安装与应用行为。保存系统版本、模块路径/hash、配置快照、host/A/B 原始结果、驱动版本及测试限制。范围外行为的明确拒绝可以满足限定契约，范围内行为标 Partial/Unverified 不能算通过。

## 风险、阻塞与停止条件

| 风险 | 必须解决的门槛 | 失败后的处置 |
| --- | --- | --- |
| Loader/入口门控 | 不在 loader lock 死等、不放行未经确认用户代码，挂起语义真实 | 更换已验证 bootstrap，保持该项未完成 |
| 驱动交付 | WDK、证书/Partner Center、适用签名流程、minifilter altitude、测试环境 | 保存具体外部依赖；可继续无驱动阶段，不标 Container 完成 |
| COW/Registry union | 对象身份、mapped I/O、tombstone、枚举/崩溃一致性 | 缩小明确支持矩阵或调整 backend，不能把写失败改成 Host fallback |
| 代执行/IPC | 真实请求者与目标覆盖，宿主服务不受全局影响 | 不支持的应用阻断；若需求必须覆盖则另选更强 backend |
| Lazy host 变化 | 对照宿主更新和应用运行，明确不冻结宿主 | 产品说明与导出语义保持一致 |
| 内核稳定性 | Verifier、dump、长期/并发、卸载与其他 filters | 停止推广，保留可恢复 Compatibility 入口 |

不写未经测量的时间或内存承诺。原型后按实际工作量估算阶段成本；完整路线是系统组件工程，不作为一次 Runtime 小修发布。

## 持久记录与后续执行

本目录是后续恢复入口：[总规格](2026-10-06-isolation-spec.md)定义用户契约，本文件定义完整顺序及终验，[研究](../../container-feasibility/research.md)保存依据，[DNS 规格](../../dns-transports/spec.md)保存协议细节。后续开始实施时按 P0 起生成有依赖的具体工作项；不要先把 P8 标完成，也不要只记 V1。

当前只写规划，不创建实现分支、安装驱动或发布。用户要求的长期终点已经纳入规格；签名申请、对外写入、安装部署及数据清除按实际操作时的授权边界处理。项目当前“不提供安全边界”“不提前做驱动”的既有规则仍适用于现有 Compatibility；进入后续阶段前以本规划明确的条件推进，并同步记录正式产品契约决定，不能悄悄改变既有默认行为。
