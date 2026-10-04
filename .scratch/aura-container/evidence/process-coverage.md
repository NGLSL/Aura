# 进程入口覆盖与门控证据

Date: 2026-10-04
Status: partial-validation

本表属于票 05 的进展，不表示票已完成。Legacy 兼容启动、入口时序、Runtime 身份和整棵进程树覆盖分别验收。运行日志位于被 Git 忽略的 target；最终验收必须以固定 Runtime bundle 刷新，不能用正在重建的 DLL 得出稳定结论。

| 目标或入口 | 已知状态 | 证据与限制 |
| --- | --- | --- |
| 原生 console entry fixture x64/x86 | 已有真实门控正负向样本 | gate64-green/gate32-green：成功入口与 release ACK；错误 bundle、断连、idle Host、TLS 拒绝均无入口 marker |
| 标准 MSVC CRT console x64/x86 | 已有公开 Session API 正向样本 | gate-public64/gate-public32：实际 staged Runtime、generation、handshake_ok、入口 marker；后续修改需固定 bundle 重验 |
| GUI entry/标准 CRT x64/x86 | 五类入口实际验证通过 | 最终候选 bundle 的 public gate64/32 覆盖无 CRT、返回式 PE、CRT console、无 CRT GUI、CRT GUI；原先失败日志保留，不替代未测应用兼容性 |
| EXE TLS callback | 当前显式门控 Unsupported | Runtime 保守拒绝非零 callback pointer；实际 TLS fixture 入口及 TLS marker 均未执行 |
| 本机 cmd.exe x64/x86 | 四组真实中间进程通过 | root/child 架构四组合、完整 DTO、child ACK、真实 Job 与 PID generation；Stop 只结束自己的 Job |
| 本机 Windows PowerShell x64/x86 | 显式门控 Unsupported | EXE TLS callback pointer 非零；legacy compatibility 另计 |
| Rust envbox-probe x64 | 显式门控 Unsupported | 当前 debug EXE 含 TLS callback pointer；legacy 真实注入 Probe 不替代门控证明 |
| Rust envbox-probe x86 | legacy DNS 真实注入通过 | 已安装 Rust 1.99 i686 target 并构建 Probe；strict/Host 对照与 typed DTO 分别验证，不等于支持 EXE TLS 门禁 |
| CreateProcess A/W 子树与跨架构 | 24 组合真实矩阵通过 | 父/子架构四组合 × A/W/AsUser × 原始挂起开关；绑定 ACK、generation、sealed bundle、入口 ACK 与实际 Job 分别确认，日志 child-final-matrix-* |
| CreateProcessAsUser | 当前用户 token fixture 通过 | 已纳入上述矩阵；不推断另一用户/完整性级别/提升权限兼容性 |
| WithToken/ShellExecute/Native/WMI/服务代执行 | 完整门控保证 Unsupported，普通行为未验证 | 当前适配不包含这些路径；不得宣称完整进程树 |
| Packaged/AUMID | PostActivation/Partial；显式 EXE gate Unsupported | early-start 与已有 PID 复用需专门实机矩阵 |
| Chromium renderer/WebView sandbox | 未验证 | 不修改 sandbox；主进程注入不证明 renderer 注入与身份 |

## 本机静态 PE 样本

采集系统版本为 build 26200.9457、25H2；shell x64、非管理员。注册表 ProductName 沿用的 Windows 10 Education 字符串不能代替精确 build。

```text
System32/cmd.exe              machine=8664 subsystem=3 TLS RVA=0
SysWOW64/cmd.exe              machine=014C subsystem=3 TLS RVA=0
System32/.../powershell.exe   machine=8664 subsystem=3 TLS RVA=86216 callback VA=5368796952
SysWOW64/.../powershell.exe   machine=014C subsystem=3 TLS RVA=5860 callback VA=4200720
target/debug/envbox-probe.exe machine=8664 subsystem=3 TLS RVA=592256 callback VA=5369210624
```

静态事实只能说明当前拒绝规则或待测候选。TLS pointer 指向空数组也被当前实现保守拒绝；不能改写成“所有 TLS directory 都被拒绝”。导入 DLL 初始化不属于 EXE entrypoint 门控覆盖范围，内核策略仍须在 loader 执行前绑定。

## 尚缺证据

其他启动入口的实际权限/结果；Packaged 冷启动、并发激活与已有 PID 的同/异 Profile 冲突；真实浏览器 renderer；安装后的共享 GUI/CLI 能力展示。Packaged 现按激活调用前 FILETIME 保守拒绝旧 PID、generation 不可读时拒绝，但尚未证明激活窗口内其他调用创建进程的所有权。当前记录不能关闭票 05，也不能满足完整 Container 的 F01/F10。
