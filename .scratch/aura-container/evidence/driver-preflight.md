# 21：驱动实验环境预检

检查日期：2026-10-04。范围：当前 Windows 工作站，只读环境盘点。票据状态由主 Agent 管理。

**报告已完成；22 号票的实际驱动加载资格尚未具备；43/44 号票的正式签名与安装资格尚未证明。** 这三个结论独立。缺正式材料不阻止具备实验条件后的隔离原型；本轮缺 WDK 和明确测试目标，不能执行加载实验。

## 可复现入口与证据

在项目根目录执行以下命令；脚本不提权、不安装组件、不申请证书、不修改启动配置，也不创建测试证书：

```powershell
New-Item -ItemType Directory -Path target/container-driver-preflight -Force | Out-Null
& .\tools\container-driver-preflight.ps1 |
    Set-Content -Encoding utf8 target/container-driver-preflight/inventory.json
Get-Content target/container-driver-preflight/inventory.json -Raw | ConvertFrom-Json
```

脚本：[container-driver-preflight.ps1](../../../tools/container-driver-preflight.ps1)。原始本地输出位于 `target/container-driver-preflight/inventory.json`（构建证据目录，不作为持久规格）。本报告保存关键原始结果，后续运行可能因工具安装、权限或系统升级发生变化。

`observed` 只代表成功读取；布尔值 false、原生退出码非零不代表资格通过。脚本保留 Exception/HRESULT/error ID；没有把权限失败转成“关闭”或空列表通过。证书仅统计有效代码签名证书数，不读取或导出私钥、证书身份和外部账户材料。

## 当前工作站实测

| 项目 | 原始观察 | 资格判断 |
| --- | --- | --- |
| Windows | Microsoft Windows 11 教育版；10.0.26200；build 26200；64 位 | verified：本机 x64 信息；不是测试 VM 的 OS 证明 |
| 运行身份 | Administrator role=false | verified：当前进程未提权 |
| Visual Studio | BuildTools 17.14.37628.2；`D:\Tools\VS2022BuildTools` | verified：安装版本 |
| MSVC | 工具集 14.44.35207；Hostx64/x64 `cl.exe` 文件版本 19.44.35228.0 | verified：编译器存在；不证明内核构建成功 |
| Windows Kits | `Include/Lib` 中发现 10.0.26100.0 | verified：SDK 目录；QFE 不能从 `.0` 目录推断 |
| WDK headers/libs | `km/ntddk.h`、`km/fltKernel.h`、`Lib/10.0.26100.0/km/x64/ntoskrnl.lib` 均 false | conditional：当前标准 Kits 路径缺必需 WDK 文件；未声明备用 WDK/EWDK 路径 |
| WDF / INF 工具 | Include/wdf=false；x64/Inf2Cat.exe=false | conditional：没有已验证的驱动包构建工具链 |
| SignTool | SDK x64/signtool.exe=true；文件版本 `4.00 (WinBuild.160101.0800)` | verified：签名工具存在；不证明签名材料或内核加载资格 |
| Secure Boot | `Unable to set proper privileges. Access was denied.`；HRESULT -2147024891；`SetPrivilegeFailed,Microsoft.SecureBoot.Commands.ConfirmSecureBootUefiCommand` | unverified：读取权限不足，不能声明启用/关闭 |
| Device Guard | VirtualizationBasedSecurityStatus=0；SecurityServicesConfigured=[0]；SecurityServicesRunning=[0] | verified：当前 WMI 查询结果；不能推断其他测试目标 HVCI 状态 |
| BCD | `/enum` exit=1；`The boot configuration data store could not be opened. Access is denied.` | unverified：TESTSIGNING 和启动调试状态无法读取 |
| Hyper-V feature | `请求的操作需要提升。`；HRESULT -2147024156 | unverified：未提权，不能推断未安装 |
| VM inventory | `Hyper-V Get-VM unavailable` | unverified：无可用查询命令；不能据此推断磁盘不存在 VM |
| 虚拟化/调试命令 | Get-VM、VBoxManage、vmrun、windbg、kd 均不在当前命令搜索路径 | verified：PATH 发现结果；未声称全盘不存在这些工具 |
| Code signing certs | CurrentUser/My=0；LocalMachine/My=0（当前有效代码签名证书） | conditional：无已验证测试签名材料；硬件令牌或外部证书未盘点，EV/Partner Center 资格不可推断 |
| 明确隔离测试环境 | 本轮未提供目标 ID、VM 配置、检查点、恢复记录或操作授权 | conditional：不能把宿主工作站或任意发现的 VM 当成测试目标 |

未提供 WindowsSdkDir/WDKContentRoot/WdkDir/VCToolsInstallDir 覆盖路径。本仓库预检前未发现 `.sys/.inf` 驱动交付文件。未执行最小驱动编译、INF/CAT 生成、签名、加载、Driver Verifier、内核调试、快照恢复和日志导出；以上都未算通过。Rust 用户态编译器可用也不等于内核 Rust 工具链可用。

## 下游门槛与提供人

| 目标 | 当前状态 | 尚缺证据 | 提供人及下一步 |
| --- | --- | --- | --- |
| 22：最小驱动构建 | conditional | 匹配 SDK/WDK、驱动平台工具集、headers/libs、Inf2Cat；空功能 `.sys` 与 INF/CAT 实际构建 | 实施者在授权 fixture 或明确构建环境补齐；优先沿用 VS2022 支持的匹配 WDK，完成实构建后才记 verified |
| 22：隔离加载 | conditional | 明确指定可丢弃 VM/测试机及 ID；检查点与恢复演练；内核调试、转储/事件日志导出；测试证书和目标信任策略；Secure Boot/HVCI/TESTSIGNING 的真实配置 | 用户/测试环境负责人提供目标与操作边界；实施者先验证恢复和签名，再在该目标实验；当前宿主禁止替代 |
| 22：host/A/B 归属实验 | conditional | 驱动构建与隔离加载通过；身份、PID 创建时间、退出/PID 复用、失败清理的内核观察 | 驱动实施者在隔离目标运行正反例，保留原始日志；预检报告不能解除加载门槛 |
| 43：正式签名资格 | conditional | 组织身份/Partner Center 权限、适用证书与提交资格、目标 OS 支持矩阵、HLK/WHCP 包与微软返回签名、最终签名验证 | 产品负责人提供外部资格；实施者按官方流程验证，不输出私钥/账户凭据、不代申请 |
| Minifilter 正式 altitude | conditional | 微软分配给本组织/产品的 altitude 和 load-order group | 产品负责人申请并提供分配记录；不得复制文档示例或其他产品 altitude 作为正式值 |
| 44：真实安装升级卸载 | conditional | 43 与正式 backend 资格、隔离安装矩阵、回滚/恢复证据 | 安装实施者依赖真实签名产物和已批准环境；本报告不证明安装成功 |

## 官方要求刷新

以下来源于 2026-10-04 读取的微软文档；正式发版前再次核对目标系统和签名策略。

- [WDK 下载与版本匹配](https://learn.microsoft.com/en-us/windows-hardware/drivers/download-the-wdk)：当前文档给出 VS2026/WDK28000 路径，同时保留 VS2022 对应 WDK26100.6584。SDK/WDK build number 要匹配；目录 `.0` 不代表实际 QFE。本机只有 SDK26100，不能用存在的 MSVC/SDK 推断 WDK 完整。NuGet/EWDK 是可选获取路径，本轮未下载安装。
- [Driver signing offerings](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings)：attestation 定位于测试，要求 EV 证书用于提交，不能视为零售 WHCP 资格。正式规划采用 HLK/WHCP 验证及 dashboard 签名；不能用本地自签替代。文档另有在明确预配置目标保留 Secure Boot 的 preproduction 路径，不能假定普通宿主信任此签名。
- [Test-signed code loading](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/the-testsigning-boot-configuration-option)：默认不加载测试签名内核驱动；TESTSIGNING 更改需要管理员和重启，Secure Boot 可能限制设置；HVCI 下仍需要签过的 binary。记录这些约束不授权修改当前机器。
- [Minifilter load groups and altitudes](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/load-order-groups-and-altitudes-for-minifilter-drivers)：首个整数 altitude 必须由微软分配；只有已有同组分配值时才有小数扩展路径。目标为路径虚拟化仍需证明组别选择与兼容性，不自行占用示例数字。
- [Altitude request](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/minifilter-altitude-request)：正式分配属于外部申请事项；本轮只读文档，没有发送邮件或提交请求。

## 本轮验证

- PowerShell AST parse：PASS；实际运行输出能被 `ConvertFrom-Json` 解析。
- 权限拒绝、缺失 WDK、缺失 VM 命令和证书数已实际读取并保留，没有执行 boot/feature/driver 修改。
- 文档与脚本范围检查、相对链接检查及 `git diff --check`：PASS。
- 空功能驱动构建/签名结构检查：未执行，明确由缺 WDK 阻塞；隔离加载和正式签名均未验证。
