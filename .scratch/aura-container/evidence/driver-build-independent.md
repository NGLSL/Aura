# 票 21：空功能驱动构建与签名结构预检

日期：2026-10-06。范围是当前 Windows 工作站上可独立完成的构建切片；没有安装、加载或签名驱动，也没有改变启动配置、安全策略或证书存储。

本报告把“可以生成一个真实的 x64 `.sys` 和 INF/CAT 包”与“该驱动可以在 Windows 上加载”分开记录。后者仍需要明确的隔离 Windows 环境、测试信任路径和后续内核验证。

## 可复现入口

从仓库根目录运行：

```powershell
& .\tools\build-empty-driver.ps1
```

脚本只在 `target\driver-build-independent\` 下恢复 NuGet 包、展开本地构建输入并写入日志。它不调用 `pnputil`、`sc`、`bcdedit`、Driver Verifier 或任何安装/加载 API。`-NoRestore` 可以用于只使用已有的本地包缓存进行复跑。

实现文件是 [`drivers/envbox-empty/envbox_empty.c`](../../../drivers/envbox-empty/envbox_empty.c)、[`drivers/envbox-empty/envbox-empty.inf`](../../../drivers/envbox-empty/envbox-empty.inf) 和 [`tools/build-empty-driver.ps1`](../../../tools/build-empty-driver.ps1)。目标驱动只有 `DriverEntry` 和必需的 `DriverUnload`：不创建设备对象，不注册设备、事件或 IO 回调，不提供 IOCTL，不实现文件、Registry、网络或进程策略。

## NuGet 恢复锁定

构建脚本按 Microsoft WDK NuGet 文档的依赖路径恢复以下三个包，版本固定为 `10.0.26100.6584`，并在展开前验证文件大小和 SHA-512：

| 包 | 大小 | SHA-512（hex） | 结果 |
| --- | ---: | --- | --- |
| `Microsoft.Windows.WDK.x64` | 110,872,506 | `8E175D6819E1303AADDC656BDF64554ED691D0A1E66438D8D09093327D74390A64E3E01285708AF50034F6F05ACE9F278B5323BBCE2A3394865DF21F9F2389FA` | verified |
| `Microsoft.Windows.SDK.cpp.x64` | 52,245,405 | `FB913010BC0EBEC4B3806AC70D0D2CB5D68EB5864719F27D72FC7D6CDE83F3C2B3394F892EC14BD5B10A1382BB53491DF7DAEF6BFBAFD4FE5A0EF41644283B39` | verified |
| `Microsoft.Windows.SDK.cpp` | 160,036,542 | `2AB1D73514F4B2BDC1AA6BD5062AF467F72BD48EBA32F999E984F005ACE2D13CFB3F7E7AB91082CA78E5DFFAD9C2C4B424B602743FFE7E54D2375D35ADC9A6FA` | verified |

恢复关系与 Microsoft 的包说明一致：WDK x64 依赖 SDK CPP x64，后者固定依赖相同版本的 SDK CPP。包只展开到项目 `target`，没有安装到 Windows Kits，也没有使用包内任何测试证书或导入证书。

官方入口：[Install the WDK using NuGet](https://learn.microsoft.com/en-us/windows-hardware/drivers/install-the-wdk-using-nuget)、[Download the WDK](https://learn.microsoft.com/en-us/windows-hardware/drivers/download-the-wdk)、[Microsoft.Windows.WDK.x64 10.0.26100.6584](https://www.nuget.org/packages/Microsoft.Windows.WDK.x64/10.0.26100.6584)。

## 主机和构建结果

本次实际构建使用：

| 项目 | 实际观察 |
| --- | --- |
| Windows | Windows 11 x64，主机 build 26200 |
| Visual Studio Build Tools | `D:\Tools\VS2022BuildTools`，MSVC `14.44.35207`，compiler/linker file version `19.44.35228.0` |
| WDK/SDK | NuGet `10.0.26100.6584`，kit layout `10.0.26100.0` |
| 目标 | x64 WDM native `.sys` |
| 编译 | `cl.exe` exit `0` |
| 链接 | `link.exe` exit `0` |
| INF 目录签名性检查 | `Inf2Cat.exe /driver:<output> /os:10_X64` exit `0` |
| INF 规则检查 | `InfVerif.exe /w <envbox-empty.inf>` exit `0` |

本次产物观察值：

| 产物 | 大小 | SHA-256 |
| --- | ---: | --- |
| `envbox-empty.sys` | 1,536 bytes | `9E9219AB9D9F6350814A329F470A9DD512B127D2A5199F73BD0CA0A1C3E81AB1` |
| `envbox-empty.cat` | 1,136 bytes | `EA963EEE984EB3799A00530828F5B55FDA0801F0E94D21DFF8F730462EA11C5E` |

链接器生成的 PE 时间戳会随每次构建改变，所以 `.sys` 的 SHA-256 是这次运行的观察值；NuGet 输入包的固定 hash 才是恢复完整性门禁。机器可读结果和原始日志位于 `target\driver-build-independent\result.json`、`compile.log`、`link.log`、`inf2cat.log`、`infverif.log`、`infverif-whql.log`、`dumpbin-headers.log`、`dumpbin-imports.log` 和 `signtool-verify.log`。

## PE、INF 和签名结构

`dumpbin /headers` 实际确认：

- PE machine 是 x64（`8664`）。
- subsystem 是 `Native`。
- Import Directory 为零；`dumpbin /imports` 没有 DLL/import table。
- Certificates Directory 为零。
- 入口是显式的 `DriverEntry`；没有把用户态 DLL 或 CRT 作为隐式运行时依赖。

INF 使用 Windows SDK `devguid.h` 中的官方 `GUID_DEVCLASS_SYSTEM`（`{4d36e97d-e325-11ce-bfc1-08002be10318}`），没有为 `System` 类伪造随机 GUID。包限定 x64 Windows 10 build 16299 及以后，使用 Driver Store `DIRID 13`、`PnpLockdown=1` 和 `%13%` service binary 路径；`InfVerif /w`、`InfVerif /h` 和 `Inf2Cat` 均以零退出码完成。有关 INF 版本字段和系统类匹配要求，见 [INF Version Section](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/inf-version-section) 和 [System-defined device setup classes available to vendors](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/system-defined-device-setup-classes-available-to-vendors)。

签名检查是预期的失败：`Get-AuthenticodeSignature` 为 `NotSigned`，`signtool verify /kp` exit `1`，日志明确没有签名。生成 `.cat` 只代表目录文件可以生成，**不代表 `.sys` 或 `.cat` 已签名，也不代表可以加载**。没有创建测试证书、没有导入证书、没有执行测试签名或产品签名。正式签名仍需按 [Driver signing offerings](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings) 的测试/发布路径另行验收。

## 状态与下一步

| 项目 | 状态 | 说明 |
| --- | --- | --- |
| x64 WDK/SDK 恢复和 hash 门禁 | verified | 三个依赖包实际下载、校验并展开到 `target` |
| 空功能 x64 `.sys` 编译/链接 | verified | WDM `DriverEntry` + `DriverUnload`，无策略功能 |
| INF/CAT 结构 | verified | `InfVerif /w`、`InfVerif /h`、`Inf2Cat /os:10_X64` 通过 |
| PE/import/certificate 结构 | verified | native x64、无 imports、无 embedded certificate |
| 测试签名/产品签名 | conditional | 没有证书材料，也没有伪造签名；当前产物明确 unsigned |
| 驱动安装/加载 | unverified | 本轮没有安装或加载；宿主不能作为替代测试环境 |
| Verifier、崩溃转储、Secure Boot/HVCI 矩阵、恢复 | unverified | 需要可回滚的隔离 Windows 测试环境 |
| x86/ARM64 目标 | unverified | 本切片按“先 x64”边界执行 |

因此本切片将票 21 的“空功能构建与签名结构检查”推进为 `verified`，但不关闭票 21，也不解除票 22 的隔离加载门禁。后续必须在明确的 VM/测试机上再验证测试信任路径、实际安装、加载、Verifier、故障恢复以及 host/A/B 进程归属；正式发布资格仍由签名和 WHCP/HLK 工作包负责。
