# 浏览器环境配置与检测：任务地图

Spec: [规格](./spec.md)
Research: [可行性评估](./research.md)
Date: 2026-10-16

## 任务与依赖

| 票号 | 交付内容 | Blocked by | Status | 覆盖规格验收 |
| --- | --- | --- | --- | --- |
| [01](./issues/01-chinese-region-labels.md) | 11 个地区中文显示、搜索与保存重开 | — | resolved | A11、A12；A13 的地区兼容 |
| [02](./issues/02-current-egress-timezone.md) | 当前 Aura 出口查询、完整时区映射、预览与填入 | — | resolved | A1、A3、A4；A9 的查询失败 |
| [03](./issues/03-explicit-egress-timezone.md) | 指定公网出口 IP 的时区查询与草稿填入 | 02 | resolved | A2、A3；A9 的指定 IP 查询失败 |
| [04](./issues/04-chrome-independent-data.md) | Chrome 独立数据目录与跨入口安全启动 | — | resolved | A5、A6、A7、A13 的目录兼容 |
| [05](./issues/05-claude-detection-pages.md) | 在所选 Chrome 环境打开两个固定检测页 | 04 | resolved | A7 的受控复用、A8；A9 的页面失败 |
| [06](./issues/06-integrated-runtime-acceptance.md) | 首版组合流程、退出界面后续用与最终验收 | 01、03、05 | resolved | 汇总 A1–A13；负责 A10 的 Chrome 实际操作证据 |
| [07](./issues/07-profile-proxy-and-readonly-egress.md) | 只读实际出口、Profile HTTP/SOCKS5 代理及共享路径 | 02、04、05 | resolved | 替换 A2，补 A14、A15 |

依赖图：02 → 03 → 06；04 → 05 → 06；01 → 06。06 的三个前置包含所有功能票的传递依赖，不重复列出 02、04。

01–06 已随 `446a7f4` 交付，原[验收记录](./evidence/runtime-acceptance.md)对应该提交。07 已按用户新决定移除手填 IP 并加入 Profile 代理；[本轮验收](./evidence/profile-proxy-acceptance.md)覆盖真实两种代理与启动路径。整体 review 的非 Stable Chrome 默认目录 P2 及增量发现的问题均已修复。

2026-10-16 用户确认恢复既有页面分工：Profile 只维护配置，启动／浏览器检测留在应用页，实例管理集中到运行实例页。实施与审查结果、实际窗口证据及后台服务验收缺口见[页面分工调整](./evidence/page-responsibilities.md)；不把此前接入 Profile 的运行面板继续作为需求。

## 为什么这样拆

- 每张功能票交付一个可独立演示的完整流程；不把模型、网络请求和 GUI 分成无法独立使用的水平任务。
- 时区映射随 02 交付并在真实 Profile 编辑路径使用，03 复用同一查询状态、预览、映射和保存契约，不再创建第二套编辑流程。
- 04 由一个负责人贯通 Application 偏好、持久化、普通 Launcher、Workspace/Supervisor、占用判定与启动结果，不拆分共享目录状态机。
- 05 复用 04 的目录和有效配置判定；不依赖时区查询在线，也不依赖 02、03 的 UI。
- 可独立领取不等于允许同时写同一文件。01/02 涉及共同 Profile 界面，02/04 也可能触及公共装配；实际调度按共享写区协调集成，不为文件冲突伪造业务依赖。

## 共同执行规则

- 首版行为与边界以规格为准。每次启动自动检测、第三方评分聚合、代理接管、数据迁移/删除和新增 Runtime Hook 均不属于本任务图。
- 禁止编写测试用例。各票复用已有构建、静态检查、Probe、Browser Probe、detached-session 检查与人工操作；只做与本票变化有关的验证，不在每票重复整套检查。
- 01 的旧地区标签断言冲突须单独记录，不能为满足旧断言保留代码显示，也不能把未通过的检查计为通过。
- 各功能票记录自己的实际证据与缺口，06 复用仍适用的证据，补组合行为及退出界面后的运行证据。未验证项不写成通过；必要验收缺失时任务保持未完成。
- 本任务仅维护本地票，不修改 GitHub #1。实现、审查和当前分支提交按用户授权执行；不包含推送、安装或发布。
