# Aura

在 Windows 上，为应用配置独立的语言、地区、时区和其他环境信息。

Aura 让应用直接运行在宿主系统上，同时通过环境配置控制它们读取到的部分系统信息。修改只作用于对应的进程树，不改变 Windows 全局设置。

## 安装与使用

1. 从[最新版本](https://github.com/NGLSL/Aura/releases/latest)下载并运行 `aura-setup.exe`。
2. 在“环境配置”中设置语言、地区、时区等信息。
3. 在“应用”中添加程序，选择环境配置并启动。

Chrome 的出口时区查询、独立数据目录和检测入口见[浏览器环境配置指南](docs/browser-environment.md)。

需要 PowerShell 7 时，在应用的终端宿主中选择 PowerShell 7，并先安装 `pwsh.exe`。

可在“设置”中检查更新。升级后重新启动目标应用，才能加载新的运行时；已有应用和环境配置无需重建。

## 主要功能

- 配置语言、地区、时区、环境变量及部分注册表读取视图。
- 为受支持的 Windows DNS 解析入口设置 UDP、TCP、DoT 或 DoH 上游；严格模式失败时不回退宿主 DNS。
- 可选的主机名、用户名、MAC 和 MachineGuid 读取视图。MAC 支持 `Get-NetAdapter`、`Get-CimInstance` 的本机网卡查询。
- 支持直接启动、命令提示符、Windows PowerShell、PowerShell 7 和 Windows Terminal。
- 查看运行实例与审计记录。每次启动使用固定配置，编辑只影响后续启动；关闭 Aura 后，已启动的应用继续运行。

## 使用范围

Aura 不提供安全隔离。应用保留宿主文件、网络和当前用户权限，不能把它当作沙箱。

覆盖范围取决于程序使用的读取入口；程序自行实现的解析、身份缓存和浏览器沙箱进程可能只有部分覆盖。目标程序需要管理员权限时，Aura 也需要相应权限；注入完全失败时拒绝启动。

配置保存在 `%LOCALAPPDATA%\com.aura.envbox\`。更多说明见[领域文档](docs/CONTEXT.md)和[版本记录](docs/releases/)。

## 开发

项目使用 Rust 1.99.0 和 Windows MSVC 工具链，版本由 `rust-toolchain.toml` 固定。完整安装包还需要 Visual Studio C++ 构建工具、CMake、Microsoft Detours 和 NSIS。

```powershell
# 构建 Rust 工作区
cargo build --workspace --locked

# 构建运行时和 Windows 安装包
.\scripts\build-installer.ps1
```

## 社区与许可

感谢 [LINUX DO](https://linux.do/) 社区对开源项目的支持。

项目采用 [Apache 2.0 许可证](LICENSE)，依赖许可见[第三方声明](THIRD_PARTY_NOTICES.txt)。
