# 05：在所选 Chrome 环境打开两个 Claude 检测页

Parent: [浏览器环境配置与检测](../spec.md)
Status: resolved
Category: enhancement
Blocked by: 04

## 交付行为

用户选择 Chrome Application 及实际运行环境，从 Aura 打开 IPPure 或 Net.Coffee 的 Claude 检测页，在目标浏览器中查看结果，不误用系统默认浏览器，也不改变应用以后启动的用途。

## 验收条件

- [x] 仅提供规格指定的 IPPure Claude 和 Net.Coffee Claude 两个固定页面，保持准确 URL；不新增通用供应商列表或任意 URL 设置。
- [x] 检测入口支持默认 Profile、Run With、Workspace Profile 及明确选择的 Host，并显示实际采用的环境。
- [x] 使用选定应用的启动链路、工作目录、已保存参数、04 的目录解析与环境策略，检测 URL 只作为本次临时用途传递，不写回 Application.arguments。
- [x] Workspace 的检测意图贯通既有 Supervisor 控制链路，服务端只接受两个固定站点枚举，不接受任意远程 URL。
- [x] 未有浏览器时按对应环境启动；已有浏览器只在应用、Profile/Host 和有效启动配置已确认匹配时打开新标签页。无法确认归属或存在旧配置时明确提示先关闭对应浏览器。
- [x] 不用系统默认浏览器或 Aura 内嵌网页替代目标 Chrome，不凭同一个 Profile UUID 忽略实际配置/子进程继承策略差异；Host 复用不要求 Runtime 注入证据。
- [x] 开关关闭时仍保留原有目录/参数语义，不将新增目录规则强加给旧应用；检测成功归属仍须确认，不能把外部旧窗口当受控窗口。
- [ ] 页面无法访问不停止既有浏览器或清除配置；网页负责展示结果，Aura 不汇总风险分数或据分数修改环境。
- [x] 在普通启动和 Workspace 路径实际打开两个页面，核对地址、窗口/root PID、目录和启动环境，并对照持久化参数未变化；已有实例复用与 Host 分别记录证据。

## 边界与交接

本票不依赖 02/03 的时区查询，也不采集字体、Canvas 或设备指纹。站点分数不是 Aura 启动成功或 Anthropic 官方风控结论。

对应规格 A7 的检测复用、A8 和 A9 的页面失败部分。向 06 交付实际页面/窗口关联证据，不以页面能打开替代整体环境验收。

## 实施证据（2026-10-16）

- `DetectionSite` 只包含 IPPure Claude 与 Net.Coffee Claude 两个固定枚举及规格 URL；Supervisor Detect 请求仅接受该枚举，既有 RunCommand 身份不变。
- 应用详情可明确选 Profile / Host 与站点；环境工作区通过既有已认证 Supervisor 打开检测页。URL 只进入本次 Session 请求，不修改持久 Application.arguments。
- 已有浏览器只在 lifetime lease、真实 Chrome singleton、PID 创建时间和完整有效配置全部匹配时复用；临时转交也走现有 Session 启动链路。若旧 root 退出，终止仅本次 handoff Job，避免产生脱离配置的新浏览器。
- 构建证据复用 04；实际页面、地址及普通/Workspace/Host 组合运行证据由集成验收补充，当前未宣称完成这些运行验收。

### 2026-10-16：普通检测通过，Workspace 等待启动保证决定

普通 Profile 与 Host 已实际打开固定页面并核对 root、目录和保存参数。Spec/Standards 审查发现的继承策略与检测绕过恢复门禁问题已修复并定向复核通过；Supervisor 16 项已有检查通过。Workspace 的新 Chrome 启动仍被 TLS 门禁阻塞，未将普通入口成功算作 Workspace 通过，见[组合验收](../evidence/runtime-acceptance.md)。

### 2026-10-16：Workspace 两站检测通过

用户批准兼容启动后，从真实 Profile 概览的运行面板依次打开 IPPure / Net.Coffee，均复用 Workspace root 20536。页面语言为 en-US、Intl 为 America/Los_Angeles；应用 arguments 原文未改变。IPPure 的 .html 入口由站点重定向为 /claude，Net.Coffee 为 /claude/。退出 GUI、重启 Supervisor 后旧页面仍可读取；未恢复控制权限。站点真正不可用分支没有触发，保留未验证，不把模拟缓存命中算失败验收；其余实现/运行证据见[组合验收](../evidence/runtime-acceptance.md)。
