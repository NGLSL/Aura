# 02：查询当前出口并预览、填入时区

Parent: [浏览器环境配置与检测](../spec.md)
Status: resolved
Category: enhancement
Blocked by: None

## 交付行为

用户在 Profile 编辑器点击查询当前网络出口，看到 Aura 本次查询 IP、具体 IANA 时区及对应 Windows 时区，确认后填入草稿，再用既有保存操作应用到后续启动。

本票贯通服务查询、固定版本时区数据、编辑状态、GUI、持久化与新实例读值。映射不单独拆成脱离使用入口的数据票。

## 验收条件

- [x] 当前网络查询使用规格指定的 IPPure 接口，并明确标识“Aura 查询请求的出口”；不声称已经检测 Chrome 的代理或 Claude 分流出口。
- [x] 结果展示来源、查询时间、IP、可提供的国家/城市、具体 IANA 时区、Windows 映射和状态；缺失具体时区时不可填入。
- [x] 使用随产品提供的固定版本 CLDR Windows Zones 数据及必要别名处理，完成 IANA/Windows 映射和宿主 Windows 时区存在性校验；不依赖每次联网下载映射。
- [x] 保留返回的具体 IANA 标识，不用 Windows 默认城市反向覆盖；无唯一有效映射时保留可见结果并要求手选，不使用国家、城市或 UTC 偏移兜底。
- [x] 预览中的“填入时区”仅修改两个时区草稿字段；持久化仍由既有保存操作完成，不改变地区、语言、DNS、身份和其他环境信息。
- [x] 查询在 GUI 更新线程外执行，遵守 10 秒总时限、1 MiB 响应上限和 HTTPS 证书校验；等待期间不重复派发同一请求，不自动重试或切换供应商。
- [ ] 草稿身份、来源/IP、时区或编辑会话变化以及取消编辑后，旧请求结果失效；迟到结果不得覆盖新值或另一个 Profile。
- [ ] 接口错误、超时、无具体时区和映射失败分别给出原因，保留原配置；普通启动不依赖该服务在线。
- [x] 完成实际查询、预览、填入、保存及新实例读值；使用已有 Probe/Browser Probe 或实际浏览器对照两个时区字段，确认 Host 设置及真实 UTC 时间线不变。

## 边界与交接

本票交付“当前网络出口”完整路径，03 增加指定 IP 来源并复用同一状态与映射。不得引入每次启动自动检测、后台轮询、Profile 热更新或浏览器代理读取。

对应规格 A1、A3、A4 和 A9 的查询失败部分。向 03 交付可复用的查询结果与草稿失效契约；向 06 交付实际结果、读值及未验证服务分支。

## Comments

### 2026-10-16：查询、映射、草稿流程已实现，待 GUI 与实例读值验收

- 新增 `timezone_lookup` 和 Profile 时区查询面板。当前出口固定使用 IPPure，查询在线程外执行，系统 curl 禁用默认 curlrc、仅 HTTPS、正常证书验证、总限时 10 秒；读取流另以 1 MiB + 1 字节设硬上限，超限终止请求，无重试或服务切换。
- 结果显示来源、UTC 查询时间、IP、服务地区/城市、原始 IANA、CLDR Windows 对应项及可填入状态。仅当具体 IANA 可唯一映射且当前 Windows 存在该时区时启用填入；不使用国家代表值、城市或偏移兜底。
- 映射随产品固定在 Unicode CLDR `release-48`：原始 XML、500 条 mapZone、BCP47 别名组及 Unicode License v3 位于 `crates/envbox-storage/data/`。原具体 IANA 保持不变，别名仅用于匹配；既有 Windows → 常用 IANA 编辑选择优先顺序保留。
- 填入只更新草稿的 Windows/IANA 两字段，持久化仍由原保存路径负责。来源/IP、手动时区、Profile 选择、新建/编辑/取消/删除/保存及页面导航使旧预览和迟到结果失效。
- 定向检查：`cargo check -p envbox-app -p envbox-storage --example profile_identity_view --bin envbox-app` 通过。
- 实际服务查询：IPPure 返回有效出口及 `America/Los_Angeles`（记录中不输出用户 IP）。数据检查确认该时区映射 `Pacific Standard Time`；同时核对 Indianapolis、Kolkata、Kyiv 别名及 Chatham、UTC、Manila 对应项。
- 尚待 06：实际 GUI 查询/预览/填入/取消/保存与迟到结果；新 Profile 实例 Probe/Browser Probe 双时区、UTC 时间线和未注入 Host 对照。超时、超限、证书错误、缺少时区、不可映射等服务分支只有代码处理，尚未实际触发。此票保持 claimed。

### 2026-10-16：正常使用链路验收完成

- 采用主 Agent 的[实际运行记录](../evidence/runtime-acceptance.md)：GUI 当前 IPPure 出口查询展示来源/IP、`America/Los_Angeles` 与 `Pacific Standard Time`；先将 Windows 草稿设 UTC，再查询时仍保持 UTC，点击填入后才变成 Pacific，只有保存后 TOML 改变。切换来源和关闭预览均清除结果。
- 新启动已有 Probe 的 GetDynamicTimeZoneInformation 为 Pacific、WinRT 为 America/Los_Angeles，进程及子进程正常；真实 Chrome Intl 为洛杉矶，另一个 Profile 为 UTC，未注入 Host 为上海，Date.now() 对照相差不足 40ms，宿主时区仍为 China Standard Time。此证据覆盖查询 → 草稿 → 保存 → 新实例双时区与 Host/UTC 对照。
- 本轮使用独立 fixture 补充指定来源输入校验和 IPv6 预览；正常 UI 线程保持响应，拒绝和预览均未改 Profile/TOML，详见 03。
- 固定 CLDR/别名、宿主存在性校验、线程外请求、10 秒/1 MiB、证书校验及无重试/fallback 已有代码复核与定向检查证据。
- **仍未实际触发**：迟到请求跨编辑会话、网络超时/超限/证书失败、供应商无具体时区/不可映射/格式变化。对应异常验收复选项保持未勾选，以代码复核与明确未验证记录交付；不制造服务异常。主 Agent 允许以这一记录完成本票正常使用链路，本票 resolved。
