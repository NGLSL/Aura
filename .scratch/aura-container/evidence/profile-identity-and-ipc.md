# 身份 Profile 与 IPC 拆分验收

Date: 2026-10-06
Review base: 7c04405
Scope: [当前规格](../spec.md)、[身份规格](../identity-spec.md)、实施规划 R6/R7。

## 实现与读取范围

四个可选身份字段已贯通 Core、TOML、不可变快照、Launcher、IPC、Runtime、GUI 和 CLI。缺失字段使用 Host，不自动采集宿主身份；CPU/GPU/磁盘身份与 IPv6 按用户决定延期。

| 字段 | 原生读取证据 |
| --- | --- |
| 主机名 | GetComputerNameA/W、GetComputerNameExA/W 全部格式、gethostname、GetHostNameW、COMPUTERNAME；实际 Windows hostname.exe A/B 对照 |
| 用户名 | GetUserNameA/W、USERNAME；不改变 SID/token/权限/账户目录 |
| MAC | GetAdaptersAddresses、GetAdaptersInfo、GetIfEntry/GetIfTable/GetIfEntry2/GetIfTable2，当前和永久六字节物理地址；不修改真实网卡或数据包 |
| MachineGuid | HKLM\\SOFTWARE\\Microsoft\\Cryptography，RegQueryValueExA/W、RegGetValueA/W，默认及两个 WOW64 视图；不写宿主注册表 |

同时检查空指针、短缓冲、返回长度、错误码、RegGetValue 类型过滤与 RRF_ZEROONFAILURE 的原容量边界。Winsock 未初始化仍返回 WSANOTINITIALISED；不会为了读取 Profile 主机名自行初始化或查询 DNS。

Snapshot schema 3 用四个字符串完整表示身份，空值为 Host；schema 1/2 wire 与 digest 保持兼容，不允许承载非默认新身份。身份 token 省略时保留旧 IPC 字节，启用时双向严格校验。独立 Runtime 能力 export 与实际 Hook 数量门槛阻止旧 DLL 静默忽略身份配置。

## IPC 维护性

原 ipc.rs 2205 行，拆分后入口 32 行。新模块 wire 870、profile 124、session 751、testing 33、tests 426 行。按职责维护协议编解码、领域转换和会话状态；公开调用路径 envbox_launcher::ipc 保持不变。

依赖方向 session → profile → wire，wire 不依赖转换层。消息大小、转义、严格校验、认证、恢复与状态迁移保持；测试继续在 ipc::tests。target/ipc-split-equivalence-result.json 对冻结原文件逐块比较全部非空函数体，涵盖所有编码/解码、helper、转换函数、完整 SessionTable 与原测试，仅 imports 和内部测试可见性发生调整。该拆分改善可维护性，不宣称提高吞吐或降低网络开销。

## 固定原生产物

最终双架构 Release Runtime 位于 target/profile-identity-runtime-pair-final/：

- x64 SHA256：C794A53FD1535DD5477D9C690FB7AFC071209FA56D84204E935CE1F366B945E5
- x86 SHA256：B868C6C1539A587D2E6AA00D9308CBF90B4C716E3280220E3E4C429B3B528A64

CMake/MSVC 双架构构建成功；profile-contracts 两架构各 1 项通过，覆盖身份解码/环境与浏览器子进程参数规则。

tools/test-profile-identity.ps1 在未注入 fresh WMI controller 26184 中运行，RuntimeModules=0。最终结果 target/profile-identity-final-e9e5ef198dec4ff388a273dc1e189d60/result.json：8 组/16 个父子快照全部通过。包括 x64/x86 A→B→A、64→32/32→64、被污染的继承 ENVBOX_IDENTITY_* 清理、默认 Host 和宿主前后对照。

target/profile-hostname-result.json：真实 Windows hostname.exe 分别返回 AURA-REAL-A/B，未注入宿主前后相同。这项验证暴露并修复了原先仅 Win32 ComputerName API 覆盖不足的问题。

## 桌面与浏览器

实际 Iced 编辑页截图 target/gui-identity-cb7f7b6446c94f5c89e475befc282a85/startup.png 已目视检查，四个输入与说明可见。使用独立配置根与 opt-in example，不占用用户现有 GUI singleton，不关闭已安装 Aura。编辑保存/重新加载/复制/清空用真实 ConfigStore 状态测试验证；截图只证明实际渲染，不替代鼠标手工保存、升级安装或长跑。

实际 Edge 154.0.4258.53 使用隔离 user-data-dir 和本地验收页，CDP 只读 DOM，无 JS 身份或时区覆盖。Host/A/B 的 navigator.language 和 Intl 时区作对照。另经非浏览器 Probe 父进程启动 Edge，验证 Runtime 子进程 Locale 参数继承。ICE 的 mDNS .local 候选归类为 link_local；不据此宣称公网 IP 隐藏或浏览器自带 resolver 全覆盖。

## Review

Standards 与 Spec 分别独立审阅，主 Agent 复核新 IPC 与 Probe 的跨模块契约。

Standards 首轮 2 个 P1 已修复：RegGetValueW RRF_ZEROONFAILURE 不得按更新后的 required size 清零调用方短缓冲；配置主机名不得被本机 DNS 特例豁免而走 Host DNS。前者有 canary RED/GREEN 证据，后者通过实际 Profile DNS fixture 查询计数与返回值验收。增量复核无剩余 actionable findings。

Spec 无剩余 actionable findings。浏览器非浏览器父进程 Locale 和 Windows hostname.exe 两项实际应用差异已修复并按具体入口记录。

## 最终检查

Rust 1.99.0 locked workspace build/test 全部 exit 0：467 passed / 0 failed / 33 ignored，fresh WMI controller 24572 / RuntimeModules=0，最终 DLL 为上述固定 hash。target/profile-complete-final-result.json 及 build/test stdout、stderr 保存完整结果。Launcher 单独 129 passed / 4 ignored；它们包含在 workspace 总数内，不重复相加。

另用最终产物显式执行平时 ignored 的原生 Supervisor 场景：实际不可变 Run 1/1、崩溃恢复 1/1、混合架构恢复 4/4，全部 exit 0。fresh controller 15868 / RuntimeModules=0，target/profile-complete-native-result.json 和对应 run/recovery/mixed 日志。确认实际配置与 Hook 事实、fresh 重连、PID generation 拒绝、root 存活/退出、64→32/32→64，以及 Stop A 保留 B/Host；这 6 项独立记录，不把全部 ignored 宣称已通过。

最终 Edge CDP 验收 exit 0，target/profile-browser-cdp-result.json：Host zh-CN/Asia-Shanghai、A en-US/America-Los-Angeles、B ja-JP/Asia-Tokyo，三个 ICE 均 complete/error=null，实际 .local 候选均 link_local。非浏览器父进程路径 target/profile-browser-child-final-result.json 三次 exit 0，语言/时区匹配；该 dump-dom 的虚拟时间模式 ICE timeout 不作为 ICE 合格证据，实时 ICE 以 CDP 结果为准。

变更 Rust rustfmt 检查通过，Probe host.rs/lib.rs 中未触及的格式差异在 7c04405 原文件同样存在，target/probe-format-baseline.log，未为本次新增挂接格式化无关代码。git diff --check 通过。

开发用 Debug DLL 首轮全量只有启动延迟阈值未通过（806.7ms vs 300ms）；上述最终验收改用实际 Release DLL，延迟项与全部功能项通过。不把调试构建性能结果混为交付产物性能。

## 完成边界

声明支持的身份 API、环境继承、不可变配置、DNS 严格路由、用户态恢复与产品字段构成本次范围。WMI、GetUserNameEx、Native Registry API、设备 IOCTL、程序自有缓存/解析器及任意程序的所有读取路径不在已验证矩阵；没有修改宿主权限、存储或网络出口，也不提供安全边界。

不安装驱动、不要求 VM，不把 CPU/GPU/磁盘与 IPv6 延期项计为本次未完成。发布、安装/升级、全浏览器/全应用覆盖与长跑未执行。

## 2026-10-07 产品入口合并与 UI

按用户确认统一使用“环境配置”，移除独立容器导航和手工创建/关联步骤。配置详情顶部直接选择应用运行；编辑时隐藏运行面板，保存后自动绑定稳定的内部运行作用域并刷新管理状态。旧 Container 协议、目录、历史记录和 opaque metadata 保留；记录按快照 Profile ID 聚合，停止使用原记录的真实作用域，不按可变旧关联扩大停止范围。

待确认启动按提交时 Profile 归属保留原 UUID，切换配置后仍可返回原配置查询；存在未知启动的配置禁止删除。后台生成快照后再次检查实际 Profile 身份，随后以不可变快照为准。停止全部使用 fresh List、精确 Profile/代次集合、逐实例 Stop 与最终 List；部分失败或换代保留已确认事实，不以空错误回复覆盖成功停止记录。

最终 App 单元与持久化/请求合约测试 58 passed / 0 failed / 5 ignored；App 和 opt-in example 构建、定向 rustfmt、git diff --check 通过。实际 1400/980 宽度窗口已目视检查，配置名称与状态标签分行，不再被挤成竖排，右侧滚动条留出间距。截图 target/gui-workspace-0531a98e7131419f9d13042464367ec3/startup.png 和 target/gui-workspace-6c9692376a904653b94ab2ebee7c93c0/startup.png，独立配置根自动保存作用域，无手工容器操作。

本增量的实际窗口验证覆盖渲染与绑定保存；跨配置 Stop 调度以真实 Request/Response 合约测试验证，未通过 GUI 对用户现有进程执行停止，也未将前述旧原生恢复证据计为本轮 GUI 运行验收。未更新已安装版本。
