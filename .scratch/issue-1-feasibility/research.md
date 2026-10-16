# Issue #1 外部可行性评估

日期：2026-10-15

目标 issue：[NGLSL/Aura #1](https://github.com/NGLSL/Aura/issues/1)。问题包括：按出口 IP 自动设置时区、Aura 关闭后由 Aura 启动的 Chrome 是否仍可用、不同 Chrome Profile 的关系，以及引入第三方检测验证。

本 note 只记录外部一手资料和当前仓库中与这些资料直接相关的事实；没有修改产品代码，也没有调用外部写入接口。

## 结论

可以做，但三个能力应分开定义：

1. **按出口 IP 推导时区：可做成“建议/应用”功能，不能当成精确事实。** IP 数据服务可以返回 IANA 时区；Windows 的时区字段需要可靠的 CLDR 映射。实际查询必须经过与目标 Chrome 相同的网络出口，否则 Aura 本身查到的 IP 可能与 Chrome 访问网站看到的 IP 不同。
2. **Aura 关闭后 Chrome 继续运行：当前代码结构支持这一目标；Profile 隔离要使用不同 `--user-data-dir`。** Chromium 的单例边界绑定 User Data Directory，`--profile-directory` 只是该目录下的一个 Profile 选择器。仅切换 `--profile-directory` 不能保证启动独立浏览器进程，也不能保证不同代理出口。
3. **第三方检测：可以打开受控 Chrome 访问检测站并读取页面/公开 JSON，但不能把检测站分数当成稳定验收协议。** 现有仓库内的浏览器 Probe 适合做结构化、可回归的核心验收；第三方站点适合作为可选诊断或带来源的原始报告。

## 1. IP 出口到 IANA 时区

### 已验证能力

[MaxMind GeoIP/GeoLite 响应文档](https://dev.maxmind.com/geoip/docs/web-services/responses/) 的 `location.time_zone` 明确使用 IANA Time Zone Database 名称，例如 `America/Los_Angeles`，同时返回 `accuracy_radius`。MaxMind 说明该半径只代表一个约 67% 置信度的地理范围，IP 地理定位本身不应被解释为街道或家庭地址定位；其[服务文档](https://dev.maxmind.com/geoip/docs/web-services/)也明确说明 IP 地理定位存在固有不精确性。

[IPinfo 的官方 API Schema](https://ipinfo.io/developers/ipinfo-api) 也明确把 `geo.timezone` 定义为按 IANA Time Zone Database 格式返回；该字段属于 Core/Plus/Max 等详细套餐，Lite 只提供国家和 ASN 等较粗信息。[官方示例](https://ipinfo.io/developers/code-snippets)同时提供 `lookup/me/geo/timezone` 形式的查询。

因此“服务能否返回 IANA 时区”的答案是**可以**，但应选提供 City/详细地理数据的接口，不能假设所有免费层都包含时区。

### HTTPS、凭据和限额

以 MaxMind 为例，[请求文档](https://dev.maxmind.com/geoip/docs/web-services/requests/)要求认证，API 只接受 HTTPS，并要求 TLS 1.2 或更高版本；`me` 参数返回“发出查询的 IP”对应记录。其[错误和限流文档](https://dev.maxmind.com/geoip/docs/web-services/responses/)列出了 401/402/403/404/429/5xx；GeoLite City/Country 达到每日查询限制时会返回 402，错误过多也可能触发 429。正式实现应处理超时、无记录、认证失败、配额耗尽和服务不可用，不能将查询失败静默转换成 Host 时区。

这里有一个与 Aura 的代理模型直接相关的约束：如果 Aura 进程直接请求 `me`，服务看到的是 Aura 这条网络路径的出口；如果请求由带有目标代理的 Chrome 页面发起，才有机会得到 Chrome 访问该服务时的出口。应在结果中记录“查询路径/观察到的 IP”，不要只记录一个没有来源标记的时区。

### 最小可行策略

- 在用户明确触发“从当前 Chrome 出口获取”时，在同一个受控 Chrome 网络路径中访问 HTTPS 地理服务；或由 Aura 先得到已确认的 Chrome 出口 IP，再查询该 IP。
- 保存服务返回的 IANA ID，并将国家/地区代码一并保留，作为后续 Windows 映射的输入。
- 查询失败、返回空时区、检测到代理出口与预期不一致时，保留现有 Profile，不自动覆盖人工配置。
- 把 IP 地理结果显示为“推断/建议”，提供人工确认；不以 IP 地理结果宣称用户实际所在地。

### 产品边界

首版最小流程是“检测出口 → 预览并填入 Profile”：预览应显示观察到的出口 IP、查询路径、IANA 时区和 Windows 映射，用户确认后才写回 Profile。自动流程只处理时区，不根据 IP 擅自修改语言或 Region；查询失败或出口不一致时保留原有配置。

如果后续提供“每次启动自动”模式，查询必须在生成本次不可变启动配置/RunSnapshot 之前完成，并把查询结果（或自动策略选定的结果）冻结进该 snapshot。项目约定 [README.md:23](D:/Project/Aura/README.md:23) 规定每次启动使用固定配置，编辑只影响后续启动；[run_snapshot.rs:211](D:/Project/Aura/crates/envbox-core/src/run_snapshot.rs:211) 将 `effective_profile` 克隆进 snapshot。因此运行中切换代理、出口变化或 GUI 编辑都不能热改已运行实例的时区；下一次启动再检测。查询失败时不应生成一个半自动覆盖的启动配置。

## 2. IANA 与 Windows 时区映射

[Unicode CLDR 的 `windowsZones.xml`](https://github.com/unicode-org/cldr/blob/main/common/supplemental/windowsZones.xml) 是 Windows 区域名（`other`）到 Unicode/IANA 风格区域名（`type`）的官方数据源。[LDML 规范的 Windows Zones 章节](https://github.com/unicode-org/cldr/blob/main/docs/ldml/tr35-dates.md)规定：

- `territory="001"` 是每个 Windows ID 的默认映射；
- 其他 territory 是地区特定映射；
- 同一地区可能对应多个 IANA ID，空格分隔的第一个是该地区的默认项；
- 并非所有 Unicode/IANA 时区和 Windows 时区都有一一对应项。

所以当前只维护少量常见配对的做法不足以支撑“任意出口 IP 自动设置”。最小正确实现可以选择：

- 打包经过版本标记的 CLDR 映射并按 IP 国家代码选择 territory，找不到映射就要求人工选择；或
- 复用一个明确提供 Windows/IANA 转换的系统/库能力，并记录其数据版本。

不能只用 UTC 偏移、显示名称或固定的“每个 Windows ID 一个 IANA ID”表来推断，因为夏令时和同一 Windows ID 的地区差异会丢失。

当前仓库已经有两套字段：[crates/envbox-core/src/lib.rs:198](D:/Project/Aura/crates/envbox-core/src/lib.rs:198) 的 `TimezoneProfile` 同时保存 `windows_id` 与 `iana_id`；[crates/envbox-storage/src/lib.rs:227](D:/Project/Aura/crates/envbox-storage/src/lib.rs:227) 明确把现有表标为“common zones only”。这使得增加可靠映射数据属于补全数据源，不需要改变 Profile 的基本语义。

## 3. Chrome Profile、User Data 与单例

Chromium 官方[User Data Directory 文档](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/user_data_dir.md)说明：User Data Directory 保存历史、书签、Cookie 和安装级状态；每个 Profile 是其下的子目录（通常是 `Default`）；`--user-data-dir` 可以覆盖 User Data Directory。

Chromium 当前源码的 [`kProfileDirectory` 定义](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/common/chrome_switches.h)说明 `--profile-directory` 选择“与首次启动的浏览器关联的 Profile 目录”，它不是新的 User Data 根目录。

单例关系由 [`chrome_main_delegate.cc`](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/app/chrome_main_delegate.cc)体现：`AcquireProcessSingleton` 接收 `user_data_dir`，成功取得单例才继续成为 Browser process；否则把命令行发给已存在的 Browser process，当前启动进程退出。Windows 实现[`process_singleton_win.cc`](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/browser/process_singleton_win.cc)也按 `user_data_dir` 查找已存在的 Chrome message window。

由此可直接得到：

- 相同 `--user-data-dir` 下的 `Default`、`Profile 1` 等 Profile 共享 Chrome 的 User Data 根和单例边界；从另一个 Profile 启动参数不等于新建独立 Chrome 进程。
- 要为每个 Aura Profile 保证独立浏览器实例，应给每个实例分配不同的 `--user-data-dir`，再在该目录内选择或创建需要的 `--profile-directory`。
- User Data 目录隔离只解决 Chrome 的本地状态/单例冲突；不同代理出口仍需在启动参数或 Chrome 网络配置中明确设置并验证。

当前仓库的代码事实支持“关闭 GUI 不主动杀掉已启动目标”的设计：

- [crates/envbox-app/src/app/window.rs:19](D:/Project/Aura/crates/envbox-app/src/app/window.rs:19) 的完成退出路径调用 `iced::exit()`；
- [crates/envbox-launcher/src/instance.rs:91](D:/Project/Aura/crates/envbox-launcher/src/instance.rs:91) 将启动交给 `session_host::start`；
- [crates/envbox-launcher/src/session_host.rs:638](D:/Project/Aura/crates/envbox-launcher/src/session_host.rs:638) 至 677 行持续检查 Job、Broker registry 成员和 root process，直到被跟踪的进程都结束；
- [runtime/src/hooks_process.cpp:589](D:/Project/Aura/runtime/src/hooks_process.cpp:589) 至 602 行在子进程 loader 运行前注册 Broker child binding。

因此生命周期结论是“代码路径上已支持，尚未由本次研究实际启动 Chrome 验证”。这与“任意 Chrome Profile 都能独立启动”是两个验收项，不能混为一谈。

## 4. 指定第三方检测站的自动化接口和限制

### IPPure `claude.html` 与 MyIP-Info API

IPPure 的[官方 MyIP-Info API 文档](https://ippure.com/MyIP-Info-API)给出了 `GET https://my.ippure.com/v1/info`，示例响应包含 `ip`、`countryCode`、`city`、`timezone`（例如 `America/Los_Angeles`）、`fraudScore` 和 `isResidential`。文档将该接口标为测试阶段并提示可能变化，因此它是首版“检测出口 → 预览并填入 Profile”的可用候选，不能直接当作长期兼容契约；IANA 时区结果仍需经过完整的 Windows/CLDR 映射。

IPPure 的 [Claude 检测页](https://ippure.com/claude.html)（当前页面可能重定向到 `/claude`）自述会检查 Claude/Anthropic 相关出口、WebRTC、DNS、浏览器 language/timezone、字体和 emoji 等设备信息。这些是该站点自己的页面检测能力；其中关于 Claude 内部风控、封禁或“官方检测机制”的解释没有得到 Anthropic 一手资料证实，Aura 不应把它们当成官方结论。

### Net.Coffee `/claude/`

Net.Coffee 的[官方 Claude 检测页](https://ip.net.coffee/claude/)公开展示“Claude AI 出口 IP”、风险/属性、DNS、WebRTC、时区和浏览器设备信息。页面加载的[官方前端脚本](https://ip.net.coffee/claude/claude-page.js?v=20260916a)显示：页面在浏览器中请求 `https://claude.ai/cdn-cgi/trace` 取得 Claude 出口 IP，再调用同站的 `/api/geoip/{ip}` 与 `/api/iprisk/{ip}`；后者当前返回包含 `timezone`、风险标志和 `trust_score` 的 JSON。本次对公开固定 IP 的只读请求也确实返回了 JSON：[geoip 示例](https://ip.net.coffee/api/geoip/1.163.243.141)返回国家/地区/城市/ISP，[iprisk 示例](https://ip.net.coffee/api/iprisk/1.163.243.141)返回 `timezone: Asia/Taipei` 等字段。

这证明它可以作为“用受控 Chrome 打开页面并收集原始诊断”或“在明确允许变化的情况下做 best-effort JSON 适配”的候选。当前没有找到单独的公共 API 文档、版本化 schema、认证/配额、限流或 SLA；前端脚本和页面接口属于实现细节，不能作为稳定自动化接口或发布门槛。页面的 Trust Score/风险判断也是 Net.Coffee 自己的评分，不代表 Anthropic 已确认的 Claude 风控结果。

主 Agent 复核同一前端脚本还发现：时区对照优先使用后端 `timezone`，缺失时使用国家代码到代表时区的 `CC_TO_TZ` 兜底，并通过 `tzIsExact` 区分来源。国家代表时区不适合自动写入 Aura，例如美国拥有多个时区；自动填入应要求具体 IP 的有效时区和 Windows 映射，缺失时保留原配置并要求人工选择。后端提供具体时区也只代表 IP 地理推断，不证明用户真实所在地。

因此两个指定站点的最小接入边界是：

1. 先以仓库已有 Probe 做结构化验收。当前 [tools/envbox-browser-probe/src/main.rs:135](D:/Project/Aura/tools/envbox-browser-probe/src/main.rs:135) 已用真实 Chromium/Edge headless 执行页面并抓取 JSON，[webrtc-intl-probe.html:34](D:/Project/Aura/tools/envbox-browser-probe/assets/webrtc-intl-probe.html:34) 已采集 `Intl.DateTimeFormat().resolvedOptions().timeZone`；runner 可把额外 Chrome 参数传入 [main.rs:153](D:/Project/Aura/tools/envbox-browser-probe/src/main.rs:153) 至 160 行。
2. IPPure API 和 Net.Coffee 页面/接口只做“检测出口、预览和原始诊断”；每个结果记录来源、采集时间、实际观察到的 IP、超时和解析状态，不让检测站分数直接改写语言/Region或阻断 Profile 启动。
3. IPPure 的测试 API或 Net.Coffee 的页面接口发生变化时，保留原 Profile 时区并显示“未验证”；只有供应商提供稳定、允许自动化使用的接口协议后，才考虑提升为机器可判定项。

## 验收边界与未验证项

已验证的是官方文档/源码所声明的能力和当前工作区的静态调用路径。以下项目本次没有执行，因此不能宣称已经可用：

- 未用真实 Aura/Chrome 代理出口调用 MaxMind 或 IPinfo，未验证当前网络环境下的实际 IP、时区结果、配额或凭据配置。
- 未在当前 Aura 构建中实际启动多个 Chrome User Data 目录，也未实测关闭 GUI 后 Chrome、Broker 和注入 Runtime 的存活关系。
- 未对 IPPure 和 Net.Coffee 做持续多版本抓取；两站公开页面的 DOM、JSON 字段、访问策略和限流可能变化。
- 未做跨供应商 IP 地理准确率基准。自动时区只能作为带来源和置信边界的建议，不能替代用户配置。

## 关键来源

- [MaxMind GeoIP/GeoLite 请求与安全](https://dev.maxmind.com/geoip/docs/web-services/requests/)
- [MaxMind 响应字段、IANA 时区、准确性和限流](https://dev.maxmind.com/geoip/docs/web-services/responses/)
- [Unicode CLDR Windows Zones 规范](https://github.com/unicode-org/cldr/blob/main/docs/ldml/tr35-dates.md)
- [Chromium User Data Directory 文档](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/user_data_dir.md)
- [Chromium ChromeProcessSingleton 源码](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/app/chrome_main_delegate.cc)
- [IPPure MyIP-Info API](https://ippure.com/MyIP-Info-API)
- [Net.Coffee Claude 检测页与前端脚本](https://ip.net.coffee/claude/)、[脚本入口](https://ip.net.coffee/claude/claude-page.js?v=20260916a)
