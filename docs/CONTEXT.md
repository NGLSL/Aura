# EnvBox 领域词汇

仅记录业务概念，不写实现细节。

## 进程与实例

| 术语 | 含义 |
|------|------|
| **Application** | 用户配置的可启动应用（名称、LaunchTarget、默认 Profile、工作目录等） |
| **Environment Profile（Profile）** | 一套可复用的环境视图：Locale、UI Language、Region、Timezone、DNS View、Environment Variables、Registry 白名单 |
| **EnvironmentSession** | 一次 Run 的控制面聚合：目标、Profile、Root/子进程集合、Package Identity、IsolationGuarantee、AttachStrategy、状态 |
| **RuntimeInstance** | 一次 Run 的运行记录；同一 Application 可并存多个实例 |
| **Process Tree Instance** | 隔离单位：Root Process 及其子进程树；不是可执行文件名 |
| **Root Process** | EnvBox 直接创建或激活的进程；后续子进程经注入继承 Profile |
| **LaunchTarget** | 启动方式：Executable / Command / Packaged（AUMID，禁止直接跑 WindowsApps exe） |
| **ActivationBackend** | 激活后端：Win32（CreateProcess SUSPENDED）/ Packaged（ActivateApplication） |
| **AttachStrategy** | 注入时机：PreExecution（挂起注入）/ PostActivation（激活后注入）/ PackageDebug（预留） |
| **RuntimeAttacher / RuntimeInjector** | 共享注入缝：向 PID 装载 envbox-runtime；与激活方式解耦 |
| **TargetCapabilities** | 目标能力标志：can_suspend / can_inject_runtime / can_create_environment_block / can_assign_job / can_track_children |
| **IsolationGuarantee** | 隔离保证级别：FullPreExecution / PostActivation / Partial |
| **early-start race** | Packaged root 在激活后、注入前可能已读到 Host 值的窗口（PostActivation 已知限制） |
| **Runtime IPC Bootstrap** | Runtime 经 Named Pipe 按 PID 向 Host 取 RuntimeProfile；ENVBOX_* 仅作 Win32 回退 |
| **Packaging** | 目标的打包模型：Win32 / Packaged Win32（Full Trust）/ AppContainer（UWP）/ PackagedUnknown。按 Package Identity 判断，不按目录 |
| **AUMID** | AppUserModelId；打包应用的激活标识（`PackageFamilyName!ApplicationId`） |
| **Injection Support** | Runtime 注入能力：Supported（可注入）/ Delayed（AttachStrategy::PostActivation 激活后注入）/ Unsupported（AppContainer / mitigation Blocking 或 Unknown 等，拒绝启动，不静默降级） |
| **Host** | 宿主 Windows 系统配置与真实时间线；EnvBox 不得修改 Host |
| **Probe（envbox-probe）** | 打印全部待虚拟化环境值的验收基准工具 |

## 环境虚拟化

| 术语 | 含义 |
|------|------|
| **Environment Block** | CreateProcess 传入的独立 Unicode 环境块；优先于 Hook 注入环境变量 |
| **ENVBOX_INHERIT_CHILDREN** | Environment Block 内部标志（`1`/`0`）：子进程是否继承 Profile；对应 Application.inherit_children |
| **Environment View** | 目标进程树读到的 Profile 环境（Locale/Region/Timezone/DNS/Env 等） |
| **Virtual timezone, real timeline** | 只虚拟化时区与本地时间换算；UTC/FILETIME/Unix timestamp/Performance Counter/Tick Count 保持真实 |
| **DNS View** | 虚拟化程序读取到的 DNS 配置（GetNetworkParams / GetAdaptersAddresses）；非透明 DNS 劫持。DnsMode：`Host` / `VirtualView` |
| **Registry Virtual View** | 仅白名单路径的注册表读值虚拟化；非完整 Registry Sandbox。白名单路径字段记为 `whitelist_paths` |
| **Runtime（envbox-runtime）** | 注入目标进程的 Detours DLL；负责 API Hook 与子进程继承 |
| **Fail Open** | 多数 Hook 失败时回退原 Windows API，兼容优先 |
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
