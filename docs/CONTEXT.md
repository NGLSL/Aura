# EnvBox 领域词汇

仅记录业务概念，不写实现细节。

## 进程与实例

| 术语 | 含义 |
|------|------|
| **Application** | 用户配置的可启动应用（名称、LaunchTarget、默认 Profile、工作目录等） |
| **Environment Profile（Profile）** | 一套可复用的环境视图：Locale、UI Language、Region、Timezone、DNS View、Environment Variables、Registry 白名单 |
| **RuntimeInstance** | 一次 Run 产生的运行实例；同一 Application 可并存多个实例 |
| **Process Tree Instance** | 隔离单位：Root Process 及其子进程树；不是可执行文件名 |
| **Root Process** | EnvBox 直接创建的进程；后续子进程经注入继承 Profile |
| **LaunchTarget** | 启动方式：完整/PATH 解析的 Executable，或 Command（含 `.cmd`/`.bat` wrapper） |
| **Host** | 宿主 Windows 系统配置与真实时间线；EnvBox 不得修改 Host |
| **Probe（envbox-probe）** | 打印全部待虚拟化环境值的验收基准工具 |

## 环境虚拟化

| 术语 | 含义 |
|------|------|
| **Environment Block** | CreateProcess 传入的独立 Unicode 环境块；优先于 Hook 注入环境变量 |
| **Environment View** | 目标进程树读到的 Profile 环境（Locale/Region/Timezone/DNS/Env 等） |
| **Virtual timezone, real timeline** | 只虚拟化时区与本地时间换算；UTC/FILETIME/Unix timestamp/Performance Counter/Tick Count 保持真实 |
| **DNS View** | 虚拟化程序读取到的 DNS 配置（GetNetworkParams / GetAdaptersAddresses）；非透明 DNS 劫持。DnsMode：`Host` / `VirtualView` |
| **Registry Virtual View** | 仅白名单路径的注册表读值虚拟化；非完整 Registry Sandbox。白名单路径字段记为 `whitelist_paths` |
| **Runtime（envbox-runtime）** | 注入目标进程的 Detours DLL；负责 API Hook 与子进程继承 |
| **Fail Open** | 多数 Hook 失败时回退原 Windows API，兼容优先 |
| **Job Object** | 用于生命周期跟踪/统计/一键停止；不是安全隔离边界 |

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
