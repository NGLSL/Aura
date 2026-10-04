# 轻量环境容器：文件、Registry 与驱动可行性研究

Date: 2026-10-04
Status: Assessment only

本笔记评估附件中的 shared-kernel、lazy host view、按 Session 文件/Registry overlay 路线。只读取当前代码和微软一手资料；没有实现、构建、加载驱动，没有修改安装或签名配置。网络/WFP 与进程归属由其他评估负责。

## 结论

**技术上可以做受限范围的环境 overlay，但完整透明的 Windows 文件/Registry union view 是新的系统组件工程，当前 Aura 尚无基础，不能作为一次现有 Runtime 修补来承诺。** 建议先验证指定 HKCU 子树和 AppData/Temp fixture，按行为门禁决定是否投资驱动。保留 Compatibility 模式可行；默认 Container 模式应等隔离、生命周期和真实应用证据齐全后再定。

附件关于 ProjFS 不能直接作为整个 C: 透明 overlay 的判断正确；关于 minifilter 更接近真实文件 I/O 的方向正确，但“Session PID? → overlay routing”省略了缓存、section、对象身份、枚举和跨边界句柄等主要工作。本文的难度判断为工程推断，不是完成时间或资源测量。

## 当前仓库观察

- `runtime/CMakeLists.txt` 构建 Detours 用户态 Runtime DLL；`runtime/src/hooks_registry.cpp` 为环境值读取 Hook，没有完整可写 Registry overlay。当前扫描 `rg --files -g '*.inf' -g '*.sys' -g '*.vcxproj' -g '*driver*' -g '*filter*'` 未发现驱动工程/INF scaffold。这是扫描范围内的源码证据，不代表机器无其他驱动。
- `runtime/src/hooks_process.cpp:604` 起明确跳过 Chromium sandboxed renderer 的第三方 Runtime DLL 注入。添加 AppContainer 不会自动消除这一兼容边界。
- 文件和注册表的透明写隔离会扩大现有产品契约；附件中的目录格式、ContainerSession 重命名和默认模式都只是候选设计，尚非已授权的业务变更。

## 文件 minifilter：官方能力与必须补齐的语义

微软说明 minifilter 在内核文件系统栈监视、修改或阻止 I/O；支持 FltMgr 的 minifilter 是新开发推荐模型。它给出拦截能力，不给出完成的 union/COW 引擎。[File system filter drivers](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/about-file-system-filter-drivers)

| 场景 | 已核查的平台事实 | 对 Aura 的推断/原型要求 |
| --- | --- | --- |
| 文件映射写 | Memory Manager 创建 writable memory-mapped section 前会触发 section synchronization；`PAGE_READWRITE/PAGE_EXECUTE_READWRITE` 会影响 oplock。[Section synchronization](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/fs-filter-acquire-for-section-synchronization2) | 不能只 Hook WriteFile 或见到写 IRP 后按当前 PID copy-up。须在 writable open/section 阶段保证独立 backing object，验证 MapViewOfFile 写与 flush、并发 host reader、映射存活期。 |
| I/O 的进程归属 | `FltGetRequestorProcessId` 返回请求线程当前 attached 的进程，未关联线程时可返回 0，未必是创建线程的进程。[FltGetRequestorProcessId](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/fltkernel/nf-fltkernel-fltgetrequestorprocessid) | 后续 paging/异步写不能仅信任实时 PID 表；需在打开路径关联 Session 与文件/stream/handle 对象，验证进程退出后的 I/O、PID 复用和句柄传递。 |
| 对象生命周期 | FltMgr 支持 file/stream/stream-handle 等 contexts，但 pre-create、post-close、paging file 等有不支持条件。[Minifilter contexts](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/managing-contexts) | 需要单独设计 pre-create 路由和 post-create 绑定，引用/释放、卸载与活动映射互锁；不是加一个 PID map 就完成。 |
| hardlink/reparse | 多个 hardlink 指向同卷同一文件，写入经其他 link 可见；junction 可跨本机卷且由 reparse point 实现。[Hard links and junctions](https://learn.microsoft.com/en-us/windows/win32/fileio/hard-links-and-junctions) | copy-up 必须明确是保持对象/链接等价还是有意打断；路径前缀检查不能当最终授权边界。跨卷 overlay 导致卷内 rename/link 语义变化，应先限制或明确拒绝。 |
| rename/link 名称 | FltGetDestinationFileNameInformation 覆盖 rename/hardlink 目的地；名称 tunneling 可使 pre-operation normalized name 失效，须 post-operation 获取正确名称。[Destination file names](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/fltkernel/nf-fltkernel-fltgetdestinationfilenameinformation) | 需要源/目标策略、replace-existing、atomic-save、失败回滚；禁止简单字符串替换。 |
| 删除与枚举 | SetInformation 负责 disposition/rename 等操作；DirectoryControl 包含枚举索引等状态。[Set information](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/flt-parameters-for-irp-mj-set-information), [Directory control](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/flt-parameters-for-irp-mj-directory-control) | union view 需要 tombstone，否则删除 overlay 后 host 旧文件会再次出现；merged enumeration 需去重、稳定游标、restart、短 buffer、过滤信息结构及目录通知。 |

还需要显式定义权限/ACL、共享访问与锁、open-by-ID、ADS、WOW64/短名/大小写、继承/duplicate 的 Host 文件句柄、网络卷支持、失败恢复。它们属于待设计和待原型行为；本轮未证明全部支持，也未声称所有场景都能透明模拟。

**有限原型验收建议：** 单一 NTFS fixture 的 host/A/B 三方对照；打开已有文件后映射写、SQLite/WAL 型 rename/replace、删除后 reopen、父目录 rename、hardlink/junction 指向边界外、枚举 restart、小 buffer、crash/reopen、Broker 中止、活动 handle/section 的 teardown。必须独立检查 host 内容/元数据，不只看应用返回值。

## ProjFS

ProjFS 是用户态 provider 把 backing store 的层级数据投影成文件目录；通知发生在 virtualization root 及其后代内。[ProjFS overview](https://learn.microsoft.com/en-us/windows/win32/projfs/projected-file-system), [Operation notifications](https://learn.microsoft.com/en-us/windows/win32/projfs/file-system-operation-notifications)

因此它适合新的虚拟目录和某些数据集原型，**不是直接按 Session 为同一个既存 C: 路径提供不同透明视图的现成 API**。这是从文档能力边界作出的推断。若采用新的 virtual root，应用路径、DLL/资源定位、已存在的绝对路径等仍需处理，不能视为附件目标已经实现。

## Registry：AppHive 与 kernel callback 不等于现成 union

`RegLoadAppKey` 加载 application hive，只能通过返回 handle 相对访问，不能以绝对 namespace 遍历；多进程各自加载同一文件可获得同一 hive 的 handle；`REG_PROCESS_APPKEY` 会限制其他调用者加载。所有键必须使用同一安全描述符，且不能 RegSetKeySecurity；关闭全部 handle 后自动卸载。[RegLoadAppKeyW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regloadappkeyw)

`RegOverridePredefKey` 只影响调用进程，将 predefined key 重映射到另一已打开的 key；文档用途是安装/注册场景。它不描述 host-miss fallback、合并枚举或整个 Native registry namespace 的隔离。[RegOverridePredefKey](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regoverridepredefkey)

因此：**private hive 可以成为存储后端，但两个 API 拼接不能自动提供“HKCU/HKLM union + 任意读取/write 隔离”。** 这是 API 契约推断。空 HKLM 根直接替换会使软件找不到既有 COM/系统配置；host fallback 则必须自行设计 key/value tombstone、查询/枚举/计数、WOW64 32/64 视图、notification、权限与 handle identity。

Windows Vista 起 registry filter callbacks 可通过 `CmRegisterCallbackEx` 分层拦截、完全处理/重定向操作、保存 key-object context 并修改输出。它能够把观察面下沉到 Configuration Manager，但仍需实现以上 overlay 语义。[Filtering registry calls](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/filtering-registry-calls)

AppHive 对 registry filter 也有特殊规则：filter 不应以 `\REGISTRY\A\` 绝对路径打开 application hive，必须正确使用 root object/relative opens。[Filtering application hives](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/filtering-registry-operations-on-application-hives)

**建议门禁：** 指定 `HKCU\Software\AuraFixture` 子树先做 host/A/B 读写删除枚举和两进程对照，再验证 Native opens/queries、WOW64、COM 与目标实际应用；通过后再评估 kernel filter。局部 Hook 原型通过不能宣称完整 Registry 隔离。

## 驱动发布：2026 当前官方规则

2026-04-14 更新的 Learn 文档明确将 **attestation signing 定位为 testing scenarios / testing purposes only**，不需要 HLK，但不是 Windows Certified、不能面向 retail Windows Update 发布；需要 EV certificate/Partner Center。正式产品建议预算 HLK/WHCP 流程，不能沿用“桌面 attestation 就足够生产部署”的旧口径。[Driver signing options](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings)

Microsoft Support 的当前 Windows Driver Policy 说明 April 2026 security update 后旧 cross-signed 驱动不再默认受信任；在 in-scope、启用策略的系统，WHCP 或 legacy allow-list 决定加载，存在 audit → enforcement 过程。不能把这一事实扩大为每台 Windows 立即同样执行；上线前须按支持的 Windows 版本、更新、CI/WDAC 状态核验。[Windows Driver Policy](https://support.microsoft.com/en-us/windows/hardware/drivers/the-windows-driver-policy)

HVCI/Memory Integrity 与签名是两个验收维度。微软要求 memory-integrity compatible drivers，HLK readiness 测试与启用 memory integrity 的完整路径功能验证；签名成功本身不是动态行为正确的证据。[HVCI compatibility](https://learn.microsoft.com/en-us/windows-hardware/test/hlk/testref/driver-compatibility-with-device-guard)

minifilter 第一个 altitude 必须由微软分配；请求文档要求预留 30 个工作日。它是外部交付依赖，不是发送请求/得到分配的证据，本轮没有对外发送任何内容。[Load order/altitudes](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/load-order-groups-and-altitudes-for-minifilter-drivers), [Altitude request](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/minifilter-altitude-request)

上线评估还需 WDK/CI 构建、INF/service lifecycle、管理权限安装、驱动升级/卸载、受影响的 AV/EDR/filter interop、Verifier、crash dump/recovery、Secure Boot/HVCI/WDAC 兼容矩阵。上述为落地工作项推断，未执行验证。

## AppContainer 与现有 Runtime

AppContainer 使用 package/capability SID 与用户权限交集限制资源，涉及文件、Registry、网络、COM 与 IPC；它是权限隔离机制，不能当成免费附加的透明 compatibility 开关。[Launch an AppContainer](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer)

第三方 DLL 能否加载还受到目标进程 signature mitigation 等影响；`MicrosoftSignedOnly` 可拒绝非 Microsoft image。[Binary signature policy](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-process_mitigation_binary_signature_policy)

不能声称所有 AppContainer 必然禁止 Aura 注入，也不能声称开启后现有 Detours 路线照常工作。应原型验证 Runtime DLL/依赖 ACL、IPC capability/security descriptor、dynamic-code/signature policies、启动时序和 Chromium 已有 sandbox。浏览器 renderer 的源码跳过路径已是现实兼容证据。不要为了注入直接关闭应用安全策略。

## 网络与系统服务代执行：主评估补充

WFP metadata 可提供 endpoint-owning PID，bind/connect redirection 可让 callout 修改连接目标；这些是连接策略/重定向能力，不自动产生独立路由表、接口、DNS 客户端和网络 namespace。[Endpoint metadata](https://learn.microsoft.com/en-us/windows/win32/api/fwpsu/ns-fwpsu-fwps_incoming_metadata_values0), [Bind/connect redirection](https://learn.microsoft.com/en-us/windows-hardware/drivers/network/using-bind-or-connect-redirection)

DoH 使用 HTTPS，DNS 消息在加密 HTTP 中传递。因此允许任意 HTTPS/443，同时只禁止非 Aura 的 53/853，无法推出“应用的所有 DNS 只能经过 Aura”。这是基于协议的工程推断。需要明确 destination allowlist/受控 proxy 等更强网络策略；通用 HTTPS proxy 仍可能转发应用自带 DoH，不能单凭 proxy-only 下结论。[DoH RFC 8484](https://www.rfc-editor.org/rfc/rfc8484.html)

Job 官方文档明确 WMI `Win32_Process.Create` 创建的子进程不自动关联该 Job。[Job objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects) 服务代执行也可能让 endpoint PID 归属于 service；不能自动从该 PID 推出原 Session，禁止为满足单个 Session 策略全局阻断共享服务、破坏宿主应用。此处是来源归属的推断与原型要求，需独立验证 WMI/COM/DNS service 路径；真正网络 namespace 是另外的 backend 选择，并非安装 WFP 自动实现。

## 阶段判断

### 当前控制面复用与新增模型

`InstanceJob` 已提供 Job 归组、统计、显式 Stop 和 named Job（`crates/envbox-launcher/src/job.rs`）；普通根进程已在挂起期间加入 Job 再 Resume（`launcher.rs`）。这可以复用，但 Job 成员身份、Runtime 加载和实际 Profile 生效是三个独立验收项。

Container 应是稳定配置/数据身份，每次启动仍创建新的 RuntimeInstance / EnvironmentSession；不要直接重命名 EnvironmentSession 或把一次 Run 的 UUID 当成持久 Container ID。当前普通 HostBroker 是进程内服务，Drop 后停止；已有独立 broker executable 不代表已实现持久 Supervisor。需要后台持有 Job / policy / overlay 生命周期、验证运行身份、GUI 断开重连和 Stop；保留现有“关闭 Aura 后应用继续运行”契约。控制通道还需调用者授权、状态版本和防 PID 复用，不能将当前合作型 IPC 直接当成安全控制面。

### 附件遗漏的隔离面

文件/Registry overlay 和 WFP 不自动隔离 named mutex、共享内存、named pipe、ALPC、COM 与单实例应用转交请求。微软明确 AppContainer 有独立 named-object namespace，full-trust packaged Win32 没有同样的隔离。[Sharing named objects](https://learn.microsoft.com/en-us/windows/apps/develop/communication/sharing-named-objects) 工程推断：若 Container A 的新进程把请求转给 Host 或 Container B 的已有进程，独立文件和 Registry 不足以保证实际执行环境。必须将 IPC/对象命名与 broker 行为放在第一阶段能力定义里，而不是最终才检查。

“Host read-only fallback”保护的是写入，不阻止目标读取宿主数据；共享 RW mounts 明确允许修改宿主文件。若目标还包括保密性或阻止读取 Host 身份，需要另行定义访问授权/拒绝策略，不能从 COW 推导。两种目标应在后续正式 spec 中分别验收。

建议顺序：当前身份/能力缺口修复 → 持久 Container + Supervisor → 统一 DNS 与小范围网络策略原型 → 指定 Registry / AppData / Temp 行为原型 → 通过 driver 发布和正确性门禁后逐步下沉。普通 IPC/broker/Native 行为矩阵贯穿每阶段，不放在末尾补救。

| 阶段 | 判断 | 下一阶段门禁 |
| --- | --- | --- |
| 现有 identity/capability/严格启动保证修复 | Go，保持当前产品边界 | 未生效不能被记录成完整成功；实际应用/子进程对照 |
| 指定 Registry 与 AppData/Temp 用户态原型 | Go，仅有界实验 | host/A/B 行为矩阵、明确未覆盖项、正确 crash/lifecycle |
| AppContainer optional compatibility 评估 | Conditional go | 目标软件能运行、Runtime/IPC 能力相容；失败不偷偷关限制 |
| kernel Registry filter 与 NTFS minifilter 原型 | Conditional go，需要独立驱动工作包 | signing/altitude/WDK 预检、fixture 和 mapped I/O/handle isolation 证明、可恢复测试环境 |
| 整个 C: 的透明 overlay / 默认 Container 产品 | 当前 No-go 直接实施 | 上述门禁通过、范围/保证/升级恢复协议明确、真实应用与长期验证 |

附件中的 20–50 MB service、每 DLL 几 MB、秒建和整体几十 MB 增量均**未在当前 Aura 架构测量**。空 metadata 的创建成本和真实应用第一次 copy-up 的 I/O/磁盘/缓存开销应分别测量；共享内核只能排除独立 guest kernel，不能推导具体占用。Host lazy view 也不是冻结快照，host 更新可能改变仍未 copy-up 的容器内容，这一点应在产品语义中明确。
