# 浏览器环境功能验收

Date: 2026-10-16
Candidate base: `fab51c045920e095583d31935281cc435b6dd354`
Branch: `dev`

## 当前结论

六张实施票已完成。中文地区、时区查询、普通 Run/Run With/Host 与 Workspace 的 Chrome 独立目录/检测、退出界面续用均有实际运行证据。Workspace 按用户确认采用明确标识的 Chrome 兼容启动，保留根 Runtime/Profile 核验，其他应用门禁不变。未实际触发的第三方服务异常与未测架构在下表保留为未验证。

所有运行使用 `target/issue-1-acceptance` 下的独立配置和浏览器数据，不使用宿主 Chrome 登录态。截图与原始日志留在忽略的 `target/` 中；本记录不包含用户出口 IP。

## A1–A13 对照

| 项目 | 当前证据 | 结论及缺口 |
| --- | --- | --- |
| A1 当前出口查询 | 原生 GUI 发起 IPPure 查询，显示 IP、来源、具体 `America/Los_Angeles` 与 `Pacific Standard Time` 预览 | 已验证正常路径；请求出口说明与实际来源一致 |
| A2 指定出口 IP | GUI 输入 `8.8.8.8` 获得对应预览；IPv6 `2606:4700:4700::1111` 实际 GUI 返回同一 IP、具体时区及可填入预览；127.0.0.1 和 URL 输入实际拒绝 | 正常路径与本地输入错误已验证；无时区、格式变化等供应商响应未实际触发，控制地址不代表用户真实 Claude 出口 |
| A3 草稿与持久化 | GUI 将 Windows 草稿设为 UTC，查询后仍为 UTC，填入后变为 Pacific；保存后 TOML 才改变。切换来源/关闭预览清除结果 | 预览、填入、保存、关闭已验证；迟到响应、超限与超时只有控制流复核证据 |
| A4 新实例与宿主 | 保存后真实 Chrome 网页 Intl 为洛杉矶，另一个 UTC Profile 为 UTC；Host 为上海；同次 `Date.now()` 相差不足 40ms。原有 Probe 新启动读取 GetDynamicTimeZoneInformation 为 Pacific、WinRT 为 America/Los_Angeles，父子进程均正常；宿主 Get-TimeZone 仍为 China Standard Time | 已验证 Windows/网页时区、UTC 时间线及宿主对照 |
| A5 环境隔离与目录稳定 | 同时存在两个 Profile root PID 25428、26356 和未注入 Host root PID 26272；目录尾分别为两个 Profile UUID 与 host。主 Profile 重启由原 PID 25592 变为 26356，目录不变 | 普通 Run/Run With/Host 已验证 |
| A6 本次环境与跨入口 | Run With 选择 UTC，默认运行仍启动洛杉矶；TOML 默认 Profile UUID 未变。Workspace root 20536 使用相同应用/Profile目录，复用该根打开两个站点 | 已验证，Workspace 采用用户批准的兼容保证 |
| A7 冲突与归属 | 手填目录冲突、普通 Run/Workspace 占用拒绝均明确失败；并发发起两种启动只有 Workspace root 20596 成功，普通入口报数据目录占用。Supervisor 重启后旧实例 TrackingLost、Stop 返回 NotControlled；原 Job 归零后 Exited | 关键冲突/竞态/重启路径已验证；Supervisor 离线期间原 Job 已消失的分支保守 TrackingLost，未实际制造 |
| A8 检测页面 | 普通 Profile/Host 两入口已核对；Workspace 两入口均归 root 20536，IPPure 页面重定向到 /claude、Net.Coffee 为 /claude/，网页 Intl 洛杉矶、en-US；保存参数无 URL | 已验证 |
| A9 在线失败 | 查询错误、结果不可填入及页面失败不控制普通启动；代码无自动重试/供应商切换 | 外部服务异常未主动制造；不计为实际故障验收通过 |
| A10 退出界面续用 | 强制退出及 Alt+F4 正常退出 GUI 后，普通两个 Profile 与 Host 均可新建页面及 renderer；Workspace root 20536 同样在 GUI 退出后创建 example.org 标签及 renderer 21060 | 普通 Profile、Host、Workspace 的真实 Chrome 已验证，未以托盘或 detached 脚本代替 |
| A11 地区中文 | 原生 GUI 展开下拉并滚动顶部/中段/底部，逐项确认全部11个中文地区；日本选择后的卡片、徽章与详情一致 | 已验证 |
| A12 搜索与保存 | `jp`、`日本` 各只命中日本；选择/保存/重新启动仍显示日本，TOML 为 JP，Locale/Windows/IANA 不变 | 已验证；实际操作还发现并修复旧 UTC 恒等替换死循环 |
| A13 老配置 | serde 缺失开关默认 false；原 CLI 的 Application nil 身份保持 chrome=None。独立旧地区 ZZ 配置实际加载后卡片/徽章/只读详情/下拉仍为 ZZ，TOML 未变 | 未知地区实际验证；旧浏览器参数兼容控制流已复核，未对所有旧配置开展迁移矩阵 |

## 实际 Chrome 归属与退出界面后读值

2026-10-16 12:08–12:09（本机时间）：

| 本次环境 | root PID | 窗口句柄 | root Runtime DLL | 网页 Intl | navigator.language | 新 renderer PID |
| --- | --- | --- | --- | --- | --- | --- |
| UTC Profile（Run With） | 25428 | 395414 | 已加载 | UTC | en-US | 25772 |
| 默认洛杉矶 Profile（普通 Run） | 26356 | 198874 | 已加载 | America/Los_Angeles | en-US | 21796 |
| Host | 26272 | 133320 | 未加载 | Asia/Shanghai | zh-CN | 20120 |

新 renderer 均创建于 GUI 正常退出之后；三个新标签的 Date.now() 分别为 1792123762521、1792123762543、1792123762558，宿主 UTC 同期为 2026-10-16T04:09:22Z。Profile 的 languages 为 en-US/en，Host 为 zh-CN/zh。

应用参数始终为 `--no-first-run --no-default-browser-check --remote-debugging-port=0 about:blank`，默认 Profile 保持原 UUID。当时将 Workspace run_panel 接入 Profile 概览，并实际渲染了应用选择和两个检测站点。2026-10-16 用户确认该入口违反页面分工：Profile 只负责配置，启动／检测在应用页，运行管理在实例页；本次页面调整撤除该面板。此前这项集成不再作为正确界面布局的验收依据。

## 构建、既有检查与审查

- `cargo check --workspace --all-targets --locked`、`cargo build --workspace --locked`：通过。
- 全套已有检查运行一次：`cargo test --workspace --locked --no-fail-fast`，484 通过、1 失败、33 忽略。唯一失败为旧 `JP` 显示断言，与中文化需求冲突。
- 用户明确批准仅同步该旧断言后，`cargo test -p envbox-app --bin envbox-app`：62 通过、0 失败、5 忽略。未新增用例。
- Supervisor 审查修复后，`cargo test -p envbox-supervisor --lib --locked`：16 通过。
- 最终兼容候选的受影响四个 crate `--all-targets --locked` check 通过；`cargo build -p envbox-app -p envbox-cli -p envbox-supervisor --locked` 通过。Supervisor lib 再次 16 通过；既有 launcher startup_gate 为 1 通过、4 个原有运行夹具项 ignored。没有重复完整套件。
- `tools/test-detached-session.ps1 -Scenario All -IncludeDeveloperTools`：Normal、Forced、ConsoleClose 通过，cmd/git/cargo/rustc/pwsh 通过。此脚本是既有会话回归证据，不替代上表 Chrome 操作证据。
- 使用保存后的 Profile 经原 CLI 启动已有 `envbox-probe.exe`，exit 0；GetDynamicTimeZoneInformation 为 Pacific Standard Time，WinRT 时区 America/Los_Angeles，UTC 为 2026-10-16 04:10:15，本地时刻为前一日 21:10:15，转换一致。原始输出 `target/issue-1-acceptance/profile-probe.log` 留在本地。
- Standards / Spec 双轴独立审查发现检测复用继承策略与恢复门禁两项问题，修复后两轴均定向确认无残余 finding。
- 用户批准的 Chrome 兼容启动、成员观测、恢复和 Profile run_panel 集成又经原两位独立审查者定向复核，Standards / Spec 均无新可行动 finding。
- 日志：`target/issue-1-acceptance/workspace-tests.log`、`detached-session.log`；GUI 截图位于同目录，地区专项截图位于 `target/issue-1-region-debug`。

## Workspace 兼容启动与恢复

初次 Workspace Chrome 在入口阶段报 `0xC0000142`。真实 Chrome PE 存在 TLS callback 数组（TLS RVA 0x3ae2f0，AddressOfCallbacks 0x1403b0b68），与既有门禁拒绝条件匹配。用户确认 Chrome 显式兼容启动后，仅 Chrome 在启动前选择非 gated 的 named Job 路径；其他应用仍 gated，没有运行失败后 fallback。没有修改 C++/Runtime DLL。

2026-10-16 12:25–12:27（Asia/Shanghai）实际结果：

- 实例 `003851e6-aae3-4b73-bcb4-47c78844eac1`、root PID 20536、Supervisor PID 20212，运行状态 Running，保证为 `chrome_compatibility_root_runtime`。CLI 与 GUI 均明确显示“根进程 Runtime 已核验，未启用入口门禁”。根环境事实 config_complete/profile_matches_snapshot 均 true。
- 同一 root 的实际 Job 成员先为 6 后随标签页增至 15，认证 Runtime 成员为 2；没有把所有 sandbox renderer 记为已注入。GUI 分开展示成员与已观测数量，并说明沙箱覆盖限制。
- Profile 概览选择已保存 Chrome 后，依次打开两个检测站；两个页面属于 root 20536，Net.Coffee 页面 Intl 为 America/Los_Angeles，language 为 en-US，languages 为 en-US/en。检测 URL 没有写入 TOML arguments。
- Alt+F4 真正退出 GUI 后，浏览器继续创建新标签与 renderer 21060。其后重启仅本次 QA Supervisor，Chrome root 保持响应且既有检测页仍读到洛杉矶/en-US；新 Supervisor 只观测旧 Job，记录 TrackingLost，Stop 明确返回 NotControlled，没有重新认领或停止旧窗口。
- 新 Supervisor 在线期间通过本人浏览器 CDP Browser.close 关闭该 QA Chrome；刷新后同一实例为 Exited、error=null，原 Job 成员为 0。
- 随后 12:27:27.336 同时发起 Workspace CLI 与普通 GUI 启动（原生点击于 12:27:28.297 完成）；Workspace 创建唯一 root 20596，普通入口明确报目录正在被 Aura 使用，没有第二个新根。对成功的新实例发起 Stop 返回 Stopped、成员为 0、exit 0，说明兼容启动的正常停止控制仍有效。

证据：`workspace-compat-run.log`、`workspace-compat-recovery.log`、`workspace-compat-lost-stop.log`、`workspace-compat-closed.log`、`workspace-compat-stop.log`、`concurrent-workspace.log`；截图 `workspace-compatible-observations.png` 与 `concurrent-start-result.png`。均在 `target/issue-1-acceptance/`。

保留的恢复限制：如果 Chrome 在 Supervisor 离线期间已完全退出且原 Job 已消失，缺少全树结束证据，记录仍为 TrackingLost；GUI 已说明此条件。在线重启后的“关闭 Chrome → 刷新 → Exited”路径已通过。尝试仅对本人验收标签模拟站点网络失败时，页面仍可由缓存/站点路径完成，没有获得真实加载失败证据，因此 A9 的站点故障分支仍记未验证。

## 当前运行环境

本次自启 QA Chrome、GUI、session host 与 QA Supervisor 均已结束；已安装 Aura GUI 与 Supervisor 已重新启动，安装路径 `D:\Program Files\Aura`。工作区源码尚未安装、发布或推送。
