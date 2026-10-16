# 只读出口与 Profile 代理验收

Date: 2026-10-16
Base: `446a7f4`
Branch: `dev`

## 交付结论

票 07 已完成。手填 IP 和来源选择已移除，出口 IP 只显示固定服务的实际响应。Profile 提供无代理、HTTP、SOCKS5 及服务器/端口；Chrome 与查询使用同一份配置。旧 None 配置保持原有网络语义，未修改 Windows 全局代理或 Runtime 协议。

模型为 `Profile.browser.proxy: Option<BrowserProxy>`，代理只包含 kind/host/port，不保存认证凭据。普通 Run、Run With、Workspace 和原始 CLI 的 Chrome 参数使用共享合成；原始 CLI 不额外分配数据目录。

## 实际 GUI

使用独立配置根 `target/issue-1-proxy-acceptance`，通过本机已有 `127.0.0.1:7897` mixed 代理验收，没有更改该代理服务配置。

- HTTP 与 SOCKS5 草稿分别成功查询到有效 IP、America/Los_Angeles 和 Pacific Standard Time，结果中 IP 是只读键值，不存在输入控件。
- 草稿从 HTTP 切换 SOCKS5 后重新查询；保存前 TOML 仍为 HTTP。保存后 TOML 为 SOCKS5，重新打开产品后概览仍显示该端点。
- 将代理端口改为 1，旧预览被清除，查询明确返回无法连接服务，没有产生直连成功结果；原保存配置保持可用。恢复端口后 SOCKS5 查询成功。
- 保留预览/填入/保存分离，不改变既有时区或其他字段。UTC Profile 的实际 Chrome 仍为 UTC，并未因代理地理位置自动热更新成洛杉矶。

截图：`http-readonly-result.png`、`socks5-readonly-result.png`、`proxy-unreachable-error.png`、`proxy-saved-overview.png`，均位于本次 target 目录。截图含实际出口 IP，仅留本地，不进入仓库。

## 真实 Chrome 与同代理出口核对

| 路径 | root PID | 唯一代理开关 | 网页 Intl | 同代理 IPPure 出口对照 | 检测复用 |
| --- | --- | --- | --- | --- | --- |
| 普通 Run / detached broker | 24588 | socks5://127.0.0.1:7897 | America/Los_Angeles | 与 socks5h 查询相同 | 两站均归原 root |
| Workspace / Supervisor | 472 | http://127.0.0.1:7897 | UTC | 与 HTTP 查询相同 | 两站均归原 root |

两条路径均加载真实 Runtime，网页 language=en-US。通过本人隔离 Chrome 的 CDP 打开 IPPure API 页面，读取真实响应后仅输出 IP 是否相同；两条路径均为 true，API 地理时区均为 America/Los_Angeles。浏览器没有使用宿主登录态。

保存的应用参数特意包含带空白的 `" -- "`。实际根命令行显示自动 `--user-data-dir` 和唯一 `--proxy-server` 均位于该终止符前；原参数及默认 Profile UUID 不变。这同时覆盖审查发现的终止符分支。

将已运行 SOCKS5 浏览器对应 Profile 保存为 HTTP 后，点击检测明确提示启动配置不同，root 24588 仍响应，没有将新代理配置转交旧根。随后仅关闭本人验收浏览器。

在应用有效开关区域加入带空白的 `" --no-proxy-server "`，Workspace 启动明确因代理参数冲突失败（root_pid=0），已运行 HTTP root 472 保持响应。恢复本次验收应用原参数后，正常 Stop 返回成功并结束该 QA 实例。

## 审查与必要修复

原提交整体审查发现的非 Stable Chrome 默认数据根 P2 已修复，详见[整体审查](./overall-review-446a7f4.md)。本机缺少 Beta/Canary/CfT 二进制，渠道修复为代码及官方目录定义核对，不计作多渠道实运行通过。

新增代理链路经 Standards / Spec 独立审查，修复了：

1. 短检测 handoff 被普通代理启动准备重新申请原根目录 lease，导致复用失败。现使用请求绑定、字段私有、仅在归属核验后构造的内部 capability，普通启动不能跳过所有权检查。
2. 代理及自动目录开关追加在 Chromium `--` 后导致被忽略。现按 Chromium 的 token 空白语义识别终止符，并将生成开关插到其前；冲突与目录识别共用同一解析规则，原 argv 不改写。
3. 实际 Workspace 验收发现严格快照原 browser allowlist 不含 proxy。现仅为合法可选 proxy 扩充严格嵌套形状，未知字段、缺字段和损坏值仍拒绝，旧快照仍可读。

修复后的相关路径已定向复核，无剩余可行动 finding；普通与 Workspace 两站复用、空白终止符、冲突和新快照均有上面的实际运行证据。

## 既有检查

- 受影响 core/storage/launcher/app/cli 的 `cargo check --all-targets --locked` 通过，包含依赖的 Supervisor。
- 最终 `cargo build -p envbox-app -p envbox-cli -p envbox-supervisor -p envbox-broker --locked` 通过。普通 GUI 使用的 detached broker 也纳入本次构建。
- 既有检查：core browser 18、storage browser 1、launcher session_request 2、core snapshot 3、storage containers snapshot 5 均通过；最终 app bin 62 通过、0 失败、5 个原有项忽略。
- 未新增测试用例或测试脚本，没有重复上一候选的完整 workspace suite。`git diff --check` 通过。

保留未验证项：代理认证不在本轮支持范围；真实多渠道 Chrome、更多 IPv6 代理端点、供应商响应格式变化、证书/超限和人为网络时序分支未运行。Run With 与 raw CLI 的新增代理组合使用共享路径完成代码复核，未另行启动一套实际浏览器，不据普通/Workspace 结果宣称这些组合已逐项实测。

本次自启 QA GUI、Chrome、broker 与 Supervisor 均已结束，已安装 Aura/Supervisor 已恢复。没有改动代理服务、安装新版或发布。
