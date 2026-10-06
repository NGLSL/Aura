# DoH bootstrap 可选与 UI 修复

Date: 2026-10-07

用户要求自定义 DoH 可只填 URL，`bootstrap_ips` 可留空。Core、GUI、CLI 序列化允许空列表；Runtime 在共同查询路径中通过同一 Profile 可直接连接的上游解析 DoH authority。显式地址覆盖自动解析，HTTP/TLS 保留原 URL 身份。没有可用种子时解析失败，不调用 Host DNS、不添加默认上游、不递归其他未引导的 DoH。当前自动发现连接地址为 IPv4；AAAA 业务查询不受影响。

## 已验证

- Core 52 项单元测试、7 项 DNS 配置测试通过；GUI 59 项通过、5 项跳过；Launcher DNS 定向 4 项通过。
- 双架构 native 路由契约各 18 组通过：同一 deadline/cancel，TXID/question，answer owner、CNAME、非法 A、无种子及拒绝 Host 回退。此层使用 transport seam，不计作实际 TLS 验证。
- `target/dns-profile-bootstrap-6aae77b836ad4439bd3a807f88c9fdf0/result.json`：实际注入 8/8 通过，受控 UDP/TCP 仅回答服务域名 A/CNAME，随后连接公网 DoH；涵盖 x64/x86 DNS65/getaddrinfo 与无种子失败。旧 DLL 同 runner 保留 RED 对照。
- 完整安装包重建后，使用实际 `artifacts/envbox.exe`、`artifacts/envbox-probe.exe` 及打包的两份 DLL 重跑 8/8 通过：`target/dns-profile-bootstrap-49d29bd72c294706bb3cee30df02d9e9/result.json`。`dns-host` 审计 0，不等于全局网络抓包无泄漏证明。
- 应用页默认列表比例 40%，配置页 36%，各自保留会话内拖动比例；应用改为紧凑选择行，启动、其他运行方式及实例入口集中到详情区。实际 1400 / 1100 像素窗口确认布局正常。DoH 证书下拉框已使用统一深色样式；连接 IP 显示为可选，解释收进 tooltip。

## 本轮交互修正

- 身份 API 范围说明收进标题 Info tooltip；移除 Cloudflare 专用快捷按钮。
- 保存包含填写中的有效 DNS 上游，不要求先点添加；无效草稿显示字段附近的校验错误。保存成功后从持久化 Profile 重建 DNS 编辑状态，避免再次保存重复添加。
- DNS 编辑器 3 项定向测试通过，覆盖空 bootstrap 直接保存、顺序保留、无效草稿拒绝、重载后不重复添加。
- 原生 GUI fixture 调用实际 ProfileSave，写入独立配置目录后再次编辑保存，确认仅保留一条 DoH 上游且 bootstrap 为空：`target/gui-identity-aeb8ba32240a4ca8ae1ea3cc230623b9`。未执行其 Supervisor Task。
- 应用页截图：`target/gui-identity-0f7523f853f248879415cbebbb7a7ee7/startup.png`（1400），`target/gui-identity-9be44a331d2b470ea15b4102f07ffe50/startup.png`（1100）。应用数据为合成 fixture，未启动目标应用。

- 输入溢出根因：vendored CPU renderer 忽略 Paragraph / Editor 的局部 clip_bounds。现在与父层裁剪求交，绘制后恢复父层 mask。窄编辑栏（1400 窗口、65% 列表比例）实际截图确认 GUID / URL 不越界，生成按钮、下拉标签和复选图标正常：`target/gui-identity-395a8315bb6c4180b7eb771b5c6ced28/startup.png`。

- `cargo test -p iced_tiny_skia --lib paragraph_text_is_clipped_to_its_local_bounds` 通过，断言边界内文字可见、边界外无像素、后续 Cached 文字正常，避免局部 mask 污染其他控件。

## 验证边界

扩展全仓宿主验收未全部通过：当前 Notepad 注入返回 `ElevationIntegrity(5)`；部分直接启动夹具选择 `target/debug` 的旧 Runtime，缺少当前身份确认。不能把定向通过宣称为全仓验收全绿。没有覆盖安装或修改宿主 DNS/证书设置。本轮真实种子覆盖 UDP/TCP；DoT、显式连接 IP DoH 与 literal URL DoH 种子为 native 契约证据。
