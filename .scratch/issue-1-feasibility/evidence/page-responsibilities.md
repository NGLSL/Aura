# 页面分工调整

Date: 2026-10-16
Base: `1f10fe7`
Branch: `dev`

## 交付内容

按用户本轮明确决定和环境配置总规格恢复页面职责：

- Profile 概览不再显示运行面板，进入、切换、保存 Profile 不再查询 Supervisor；保留原有本地运行作用域绑定与出口／时区查询。
- 应用页继续使用原有普通 Run、Run With、Host 和两个检测入口，不新增 Supervisor 前置条件。
- 运行实例页展示本次会话与 Supervisor 的运行记录，按实例 UUID 去重，统一应用筛选和运行计数。后台列表、诊断、停止及未知启动结果查询独立于 Profile 草稿。
- 单实例停止使用所点运行记录的身份；后台记录按实际运行 Profile 分组。无应用筛选且同一环境有多个可控制实例时保留“停止此环境实例”，过滤状态下不提供跨应用停止。generation、TrackingLost 与部分停止结果语义保持不变。

不修改 Runtime、Launcher、Supervisor 协议或持久化结构；不新增测试用例。

## 实际窗口与浏览器

使用独立配置 `target/page-responsibilities-acceptance`，没有使用或修改用户 Chrome 数据。

- Profile 原生窗口确认运行面板已消失；切换至另一个 Profile 后仍只有配置内容，后台连接错误不出现在配置详情中。
- 在已安装 Supervisor 与开发版制品不匹配的情况下，从应用页正常启动隔离 Chrome，root PID 20704、实例 `6120c843-bc89-4cd2-bb91-715b3350390e`。证明普通启动没有新增后台管理前置条件。
- 应用页的检测选择同时保留默认环境、其他已保存 Profile 和 Host。实际选择默认环境的 IPPure、Net.Coffee，两站均在原 root 的浏览器打开；页面地址为 `https://ippure.com/claude` 和 `https://ip.net.coffee/claude/`，应用保存参数未变化。
- 实例页显示该会话的应用、环境、运行状态及停止操作。筛选另一个应用出现正确空态，切回后原实例仍在；切换 Profile 后回到实例页，原会话仍可管理。从实例页终止本人验收进程树成功，记录变为 Exited。
- 首次实际进入空实例页发现 Iced 纵向滚动体嵌入全页 Fill 空态会崩溃。改为本页自然高度空态，重新打开并实际验证空列表、过滤空态、非空会话和停止后列表，均正常。

截图与 GUI 日志留在本地 `target/issue-1-proxy-acceptance/page-*.png` 和 `target/page-responsibilities-acceptance/gui-error.log`。Chrome 的 CDP 只读取本人隔离目录所公布的端点，没有接管用户浏览器。

## 构建与独立审查

- `cargo check -p envbox-app --all-targets --locked`、`cargo build -p envbox-app --locked` 通过；空态修复后重新构建并完成上述实际操作。
- 复用已有 app bin 检查：62 通过、5 忽略。未增加或改写现有测试用例。
- **Standards**：独立静态审查未发现可行动问题。
- **Spec**：独立静态审查未发现缺失或扩大范围的行为；确认全局管理与所选 Profile 解耦，普通启动契约保留。
- 静态审查未覆盖实际布局断言，空态崩溃由主 Agent 桌面验收发现并交原实现负责人修复，不以审查通过替代实际运行。

## 未验证项与运行状态

开发版无法通过已安装 Supervisor 的路径／制品身份校验。当前用户 Chrome root 18252 仍运行；未获得本轮切换服务的回复前保留原 Supervisor 24296，不中断其管理状态。

因此，本轮后台实例的真实列表渲染、诊断展开、单实例／环境批量停止与 pending 结果查询仅完成源码与既有检查复核，尚未在同版本 Supervisor 下实际重验。后续在用户允许切换服务或同版本安装环境中补验这些交互；本次实际验证了管理连接失败不会阻止普通启动、检测和本地实例停止。

本人验收 Chrome 已通过实例页停止，用户 Chrome 与原 Supervisor 保留。源码未安装、推送或发布。
