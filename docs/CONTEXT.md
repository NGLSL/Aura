# EnvBox 领域词汇

仅记录业务概念，不写实现细节。

## 进程与实例

| 术语 | 含义 |
|------|------|
| **Application** | 用户配置的可启动应用（名称、LaunchTarget、默认 Profile、工作目录等） |
| **Environment Profile（Profile）** | 一套可复用的环境信息配置：Locale、UI Language、Region、Timezone、DNS 配置视图与解析策略、Environment Variables、Registry 白名单、浏览器/WebRTC 策略 |
| **环境容器 / 环境工作区（Container）** | 持久保存身份和 Profile 引用，管理使用该环境信息视图启动的实例；目标继续使用宿主资源与权限，不表示文件或 Registry 写入隔离 |
| **RunSnapshot** | 一次运行的不可变有效配置，绑定 Container、Profile 与 RuntimeInstance；后续 Profile 编辑只影响之后的运行 |
| **EnvironmentSession** | 一次 Run 的控制面聚合：目标、Profile、Root/子进程集合、Package Identity、IsolationGuarantee、AttachStrategy、状态 |
| **RuntimeInstance** | 一次 Run 的运行记录；同一 Application 可并存多个实例 |
| **Detached RuntimeInstance** | Aura 控制面退出后仍继续运行的 RuntimeInstance 进程树；保持启动时的不可变 Profile，但不属于重新打开的 Aura 所维护的实例列表 |
| **Process Tree Instance** | 隔离单位：Root Process 及其子进程树；不是可执行文件名 |
| **Root Process** | EnvBox 直接创建或激活的进程；后续子进程经注入继承 Profile |
| **LaunchTarget** | 启动方式：Executable / Command / Packaged（AUMID，禁止直接跑 WindowsApps exe） |
| **ConsoleHost** | Application 的命令宿主偏好：Direct / cmd.exe / PowerShell / Windows Terminal；旧配置默认为 Direct，非 Direct 宿主仅适用于 Command |
| **ActivationBackend** | 激活后端：Win32（CreateProcess SUSPENDED）/ Packaged（ActivateApplication） |
| **AttachStrategy** | 注入时机：PreExecution（挂起注入）/ PostActivation（激活后注入）/ PackageDebug（预留） |
| **RuntimeAttacher / RuntimeInjector** | 共享注入缝：向 PID 装载 envbox-runtime；与激活方式解耦 |
| **TargetCapabilities** | 目标能力标志：can_suspend / can_inject_runtime / can_create_environment_block / can_assign_job / can_track_children |
| **IsolationGuarantee** | 隔离保证级别：FullPreExecution / PostActivation / Partial |
| **early-start race** | Packaged root 在激活后、注入前可能已读到 Host 值的窗口（PostActivation 已知限制） |
| **Runtime IPC Bootstrap** | Runtime 经 Named Pipe 按 PID 向 Host/Broker 取 RuntimeProfile；ENVBOX_* 结构化值仅作 Win32 回退。C++ 不解析 `profiles.toml` |
| **Session Registry** | Broker/Host 侧 PID → Session/Profile 映射；含子进程归属与生命周期事件 |
| **envbox-broker** | 独立 Host 进程：Session Registry + Runtime IPC 服务端；与进程内 HostBroker 共用同一协议 |
| **ENVBOX_* value fallback** | Environment Block 中的结构化 Profile 字段（locale/tz/dns/registry 等）；Broker 不可用时的 Win32 回退通道，非 TOML |
| **Process Tracker** | 会话进程集跟踪：Win32 用 Job Object，Packaged 用 PID + Package Identity |
| **Packaging** | 目标的打包模型：Win32 / Packaged Win32（Full Trust）/ AppContainer（UWP）/ PackagedUnknown。按 Package Identity 判断，不按目录 |
| **AUMID** | AppUserModelId；打包应用的激活标识（`PackageFamilyName!ApplicationId`） |
| **Injection Support** | Runtime 注入能力：Supported（可注入）/ Delayed（AttachStrategy::PostActivation 激活后注入）/ Unsupported（AppContainer / mitigation Blocking 或 Unknown 等，拒绝启动，不静默降级） |
| **Host** | 宿主 Windows 系统配置与真实时间线；EnvBox 不得修改 Host |
| **Probe（envbox-probe）** | 打印全部待虚拟化环境值的验收基准工具 |
| **envbox-browser-probe** | 网络/WebRTC 路径验收基准工具（policy 环境、本地地址分类、可选 STUN）；与 envbox-probe 职责分离 |

## Browser / Network Guard（WebRTC Privacy）

| 术语 | 含义 |
|------|------|
| **BrowserPrivacyProfile** | Profile 上的浏览器隐私配置；当前字段 `webrtc: WebRtcPolicy` |
| **WebRtcPolicy** | WebRTC/UDP 路径策略：`Host` / `PublicInterfaceOnly` / `ProxyOnly` / `Strict` |
| **Browser Policy** | Chromium/WebView2 策略产物：`--force-webrtc-ip-handling-policy` 与 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` |
| **BrowserEngine** | 显式浏览器引擎分类：Chromium / Edge / WebView2 / Electron / Unknown（禁止裸字符串匹配） |
| **Browser Guarantee** | 浏览器层保证级别：`PolicyOnly`（Balanced）/ `NetworkEnforced`（Strict，需 Network Guard） |
| **Network Guard** | Session 进程树 direct UDP 约束；Strict 的执行层。不做端口封禁、不改 Host 防火墙 |
| **Runtime Child Guard** | CreateProcess 缝上识别 Browser Engine 子进程并继承/确认 WebRTC 策略 |

产品哲学：**约束真实网络路径，不伪造检测结果**（非反检测）。

## 环境虚拟化

| 术语 | 含义 |
|------|------|
| **Environment Block** | CreateProcess 传入的独立 Unicode 环境块；优先于 Hook 注入环境变量 |
| **ENVBOX_INHERIT_CHILDREN** | Environment Block 内部标志（`1`/`0`）：子进程是否继承 Profile；对应 Application.inherit_children |
| **Environment View** | 目标进程树读到的 Profile 环境（Locale/Region/Timezone/DNS/Env 等） |
| **IdentityProfile** | 显式可选的主机名、用户名、MAC 与 MachineGuid 读视图；空字段跟随宿主，只覆盖声明的 Win32 查询，不改变账户/SID、权限、网卡、注册表或真实网络出口 |
| **Virtual timezone, real timeline** | 只虚拟化时区与本地时间换算；UTC/FILETIME/Unix timestamp/Performance Counter/Tick Count 保持真实 |
| **DNS View** | 虚拟化程序读取到的 DNS 配置（GetNetworkParams / GetAdaptersAddresses）；非透明 DNS 劫持。DnsMode：`Host` / `VirtualView` |
| **Registry Virtual View** | 仅白名单路径的注册表读值虚拟化；非完整 Registry Sandbox。白名单路径字段记为 `whitelist_paths` |
| **Runtime（envbox-runtime）** | 注入目标进程的 Detours DLL；负责 API Hook 与子进程继承 |
| **Fail Open** | 多数 Hook 失败时回退原 Windows API，兼容优先；strict DNS 的受支持解析入口禁止宿主回退 |
| **Job Object** | 用于生命周期跟踪/统计/一键停止；不是安全隔离边界 |
| **Audit Mode** | 可选观测开关（默认关）：记录进程树读取过的地域相关 API；不改变虚拟化语义 |
| **Audit Event** | 单条审计记录（JSONL）：API、pid/ppid/tid、是否虚拟化、非敏感摘要 |
| **DNS routing** | VirtualView 下对解析入口的 per-process 路由；区别于只改配置视图的 DNS View |

## 启动与策略

| 术语 | 含义 |
|------|------|
| **Run / Run With** | 用默认 Profile 启动 / 临时指定 Profile 启动（不改默认） |
| **Startup Fail Policy** | Runtime DLL/Profile 缺失或损坏、无法创建进程或注入时启动失败；禁止静默降级为普通启动 |
| **caller_requested_suspended** | 调用方本身要求 CREATE_SUSPENDED 时注入后不得自动 Resume |
| **InstanceStatus** | Starting / Running / Stopping / Exited / Failed |
| **Process-scoped** | 所有修改仅属于一个 RuntimeInstance 的进程树 |
| **Host-transparent** | 不修改 Windows 全局配置 |
| **Environment-consistent** | 同一 Profile 下 Region/Locale/Language/Timezone/DNS/Environment 尽量逻辑一致 |

## 隔离分层（文档概念，非代码）

| Tier | 目标 | 注入时机 | IsolationGuarantee |
|------|------|----------|--------------------|
| Tier 1 | 原生 Win32 | PreExecution（挂起注入） | FullPreExecution |
| Tier 2 | Packaged Win32 mediumIL | PostActivation（AUMID 后注入） | PostActivation（有 early-start race） |
| Tier 3 | AppContainer / 受保护进程 | 不支持（Fail Closed） | — |
