# 04：Chrome 独立数据目录贯通全部应用启动入口

Parent: [浏览器环境配置与检测](../spec.md)
Status: resolved
Category: enhancement
Blocked by: None

## 交付行为

Chrome Application 可选择使用独立数据目录。不同 Profile 可同时打开独立 Chrome，同一应用和环境重启后保留相同目录；普通 Run、Run With、Host 和 Workspace 遵循同一规则，不把浏览器复用误报为新的受控实例。

本票由单一负责人贯通启动偏好、持久化、共享参数合成、目录占用与实例归属，避免把同一状态机拆给多个并行修改者。

## 验收条件

- [x] 受支持的 Win32 Chrome Executable Application 提供独立目录偏好；旧配置默认关闭，非支持目标不得启用，既有手填参数保持原语义。
- [x] 目录位于 Aura 用户数据区，按 Application UUID 与本次有效 Profile UUID 稳定分配；Host 单独一份。重命名不换目录，不同 Profile 不共用目录。
- [x] 普通 Run、Run With 和 Workspace/Supervisor 共用根启动参数合成规则；相同应用/环境目录一致，Run With 不改变默认 Profile，Container 不额外分配第二套目录。
- [x] 手填 user-data-dir 与开关冲突时明确拒绝启动；profile-directory 保留并解释为独立根下的子配置，不把它当成独立根。
- [x] 目录被已有 Chrome 占用时，不将命令行转交误报为新受控 RuntimeInstance；提示关闭对应浏览器，不强杀、接管或修改旧实例。
- [x] 普通 Run/Workspace 并发请求同一目录，最多一个请求创建新受控根进程；占用判定覆盖实际启动结果及竞态，不只依赖单次启动前扫描。
- [x] Aura 重启或缺少旧实例记录时，不据目录名、进程名或新启动器注入成功推断旧窗口归属。
- [x] 为后续检测用途提供可核对的实际应用、Profile/Host 身份及有效启动配置；包含 Profile 内容、浏览器参数、工作目录和子进程继承等策略，不只比较 Profile UUID 或某一路径独有的记录 ID。
- [x] Host 保持未注入语义；原始 CLI 命令保持显式参数行为，不为 nil Application 身份隐式分配目录。
- [x] 不复制宿主浏览器数据；关闭开关、退出 Aura、删除 Application/Profile 都不自动删除目录。新目录第一次启动表现为新浏览器数据。
- [x] 用实际 Chrome 对照窗口/root PID、chrome://version、目录和 Runtime/审计归属，验证不同环境并存、重启稳定、两条启动链路及冲突处理；目录资源不绑到 GUI 存活。

## 边界与交接

本票不新增检测按钮或 URL 传递，不覆盖 Edge、Electron、WebView2、Packaged 或任意原始 CLI 命令。两张不同入口的表面 UI 不能替代共享启动契约。

对应规格 A5、A6、A7 和 A13 的目录兼容部分。向 05 交付共享目录/归属核对能力，向 06 交付启动证据和仍缺少的运行场景；不重写已有会话生命周期架构。

## 实施证据（2026-10-16）

- 已接入 Application `chrome_independent_data`（缺失默认 false）、编辑开关、共享 Launcher/Supervisor 参数合成；目录按 Application UUID 与有效 Profile UUID / Host 分配，不按 Container 再分配。
- 共享 `chrome::ChromeLease` 由 detached session host 或 Supervisor 的 SessionHandle 持有，Windows 文件共享模式排斥同目录并发启动；原始 CLI request 保持 `chrome: None`。
- 实际根归属同时核对 Chrome_MessageWindow 数据目录窗口、root PID 与进程创建时间；启动失败/转交不能登记为新实例。检测复用比较完整 Profile、启动参数、工作目录和继承/审计策略。
- 定向构建：`cargo check -p envbox-app -p envbox-cli -p envbox-supervisor --all-targets` 通过；后续 handoff 竞态修复的 `cargo check -p envbox-launcher -p envbox-supervisor` 通过。
- 实进程探针：独立临时目录启动真实 Chrome，spawn PID 5656，Chrome_MessageWindow 精确匹配该目录，窗口 owner PID 5656。仅关闭本次实例。此证据验证占用探针，不替代 Aura 注入与组合运行验收。
- 最终 GUI / Workspace / 并发 / 退出界面运行证据由集成验收补充，当前不据静态实现勾选运行验收项。

### 2026-10-16：普通入口运行通过，Workspace 门禁阻塞

实际普通 Run/Run With/Host 同时运行、目录稳定及默认 Profile 不变均已验证；手填目录冲突返回 root_pid=0 的明确失败，三个既有浏览器继续响应。Workspace 使用与普通 Run 相同目录并正确拒绝占用，但关闭普通浏览器后的新启动被既有 TLS 入口门禁拒绝。用户对 Chrome 显式兼容启动的决定尚未返回，本票保持 claimed，详见[组合验收](../evidence/runtime-acceptance.md)。

### 2026-10-16：用户批准 Chrome 兼容启动

用户已确认采用仅 Chrome 的显式兼容启动。实施继续贯通根 Runtime 身份核验、独立保证记录、真实成员证据与 Supervisor 重启后 TrackingLost；其他应用门禁保持不变。相应契约已纳入规格，完成定向审查和实际 Workspace 验收后再关闭本票。

### 2026-10-16：Workspace、竞态与恢复验收完成

Workspace root 20536 使用普通入口相同应用/Profile目录，根 Runtime 与完整配置核验成功；两种入口并发时仅产生 Workspace root 20596，普通入口明确拒绝占用。Supervisor 重启后旧浏览器仍响应，实例 TrackingLost 且 Stop 返回 NotControlled；新 Supervisor 在线时关闭 Chrome、刷新后 Exited/error=null。正常受控兼容实例的 Stop 返回 Stopped、Job 成员归零。两轴定向审查无新问题，本票 resolved；[组合验收](../evidence/runtime-acceptance.md)保存具体实例、日志和恢复限制。
