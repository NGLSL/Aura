# Profile 身份读视图

Status: ready-for-human
Implementation status: 已实现并完成声明入口验收；见[身份与 IPC 证据](evidence/profile-identity-and-ipc.md)。
Date: 2026-10-06
Parent: [环境信息容器](spec.md)

用户确认先实现主机名、用户名、MAC 与 Windows 安装标识 MachineGuid；CPU、GPU、磁盘身份后置。全部字段可选，缺失表示宿主读视图，不自动采集或保存宿主身份。保持真实 Windows 账户、权限、网卡、系统注册表与网络出口。

## 模型与一致性

`EnvironmentProfile.identity: IdentityProfile` 包含四个 `Option<String>`：

| 字段 | 约束 | 支持的读入口 |
| --- | --- | --- |
| computer_name | ASCII DNS 单标签，1–15 字符，字母数字或中横线，首尾字母数字 | GetComputerNameA/W、GetComputerNameExA/W、gethostname、GetHostNameW；COMPUTERNAME |
| user_name | ASCII 1–64 字符，字母数字、点、下划线、中横线 | GetUserNameA/W；USERNAME |
| mac_address | 大写六字节冒号分隔，单播、非全零 | GetAdaptersAddresses、GetAdaptersInfo、GetIfEntry、GetIfTable、GetIfEntry2、GetIfTable2；仅覆盖六字节物理地址，保留适配器数量、索引和实际网络状态 |
| machine_guid | 小写规范 UUID，非 nil | HKLM\\SOFTWARE\\Microsoft\\Cryptography 的 MachineGuid：RegQueryValueExA/W、RegGetValueA/W |

主机名 Ex 的 DNS domain 和 physical DNS domain 返回空字符串，FQDN 返回配置单标签，避免拼接宿主 DNS suffix。MAC 配置用于返回的每个六字节物理地址；不改变真实数据包。用户名读视图不改变 token、SID、账户目录或 impersonation 权限。GetUserNameEx、WMI、Native Registry API、设备 IOCTL、应用自行读取或缓存身份暂不属于支持入口，必须显示限制。

配置与 Environment 中同名值冲突时拒绝；保留 `ENVBOX_IDENTITY_*` 名称空间，不允许用户通过自由环境变量注入。Launcher 与 Runtime 子进程合并均清理整个继承身份名称空间，再从不可变 Profile 重写。启用字段不能靠父进程残留变量决定。

## 传输与兼容

新快照使用 Profile schema 3，完整记录 identity 四个字符串字段，空字符串表示 Host（领域模型仍为 Option，避免 TOML 不支持 null）。旧 schema 1/2 必须保持原始 wire value 与摘要；旧 schema 不允许携带非默认 identity。旧 TOML Profile 缺 identity 正常加载为 Host。

IPC 四个可选 token 按 computer_name、user_name、mac_address、machine_guid 顺序编码；默认 identity 不添加 token，保留旧 IPC 配置 SHA。Rust 与 C++ 双向完整校验，不接受重复、未知字段、非法值或静默截断。实际 Runtime identity 上报必须与快照一致。

新增独立 Runtime 身份能力 DATA export；启用 identity 时，旧 Runtime 在启动前明确拒绝。实际 Hook 安装失败不得声称身份视图已覆盖。默认 Host Profile 保持旧 Runtime 兼容。

## 验收

- 模型非法值、环境冲突、旧 Profile 读取、新配置保存及清空回 Host。
- 旧 schema 1/2 快照 digest 不变；schema 3 完整性与不可变配置校验。
- ENV 继承污染清除、IPC 双向严格校验、旧 DLL opt-in 拒绝。
- 双架构 Runtime 构建，新增读取入口扩展 Probe；Host/A/B 输出、A/W 与短缓冲错误/长度契约、同一 Profile 子进程一致、宿主前后不变。
- GUI/CLI 实际显示可选字段和能力限制，真实 Win32 与多进程应用按读取入口记录证据；浏览器 renderer 独立评估，不能由 DLL 安装数量推断合格。

API 缓冲语义依照 [GetComputerNameEx](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getcomputernameexw)、[GetUserName](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getusernamew) 与 [GetIfEntry2](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getifentry2) 的官方契约。
