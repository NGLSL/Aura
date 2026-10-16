# 01：国家和地区中文显示与搜索

Parent: [浏览器环境配置与检测](../spec.md)
Status: resolved
Category: enhancement
Blocked by: None

## 交付行为

用户在环境配置的地区下拉框、已选值、卡片和详情中看到一致的中文名称，可按中文名称或 ISO 代码搜索；保存后重开仍然如此，底层配置标识不变。

## 验收条件

- [x] 当前 11 个地区全部按规格显示：中国、美国、台湾、新加坡、日本、菲律宾、英国、德国、韩国、玻利维亚、土耳其，不再将 TW/SG/JP 等代码用作这些选项的名称。
- [x] 下拉选项、选中值、卡片和只读详情共用中文标签来源，地区字段标题显示“地区”。
- [x] “日本”和不区分大小写的 JP 命中同一选项；选择、排序和保存仍以代码识别，不以中文名称充当存储值。
- [x] 保存与重启后显示一致，TOML 和 Runtime 输入仍使用 ISO 地区代码；Locale/UI Language 和 Windows/IANA 时区标识未因显示改名而变化。
- [x] 旧配置无需迁移；未知旧地区代码保持可见原值，不被隐藏、默认成中国或静默改写。
- [x] 通过实际 GUI 查看全部选项、搜索、卡片和详情，并对照保存值；记录与本改动直接相关的已有静态检查结果。
- [x] 单独记录既有“JP 显示 JP”断言与新需求的冲突；遵守禁止编写测试用例的约束，不以旧断言削弱中文显示需求。

## 边界与交接

不扩展地区列表，不翻译用户自定义名称，不自动调整语言或时区。对应规格 A11、A12 和 A13 的地区兼容部分。向 06 交付实际界面与保存值证据。

## Comments

### 2026-10-16：实现完成，待界面与保存重开验收

- `options::region_label` 统一提供 11 项中文名称，未知代码保留原值；地区下拉继续通过既有 `option_label` / `filter_options` 按代码和显示名搜索。
- 环境配置卡片、地区徽章和只读详情已切换到共享标签来源；编辑字段标题改为“地区”。保存模型和 Runtime 输入仍使用原代码，未新增数据迁移。
- 定向检查：`cargo check -p envbox-app -p envbox-storage --example profile_identity_view --bin envbox-app` 通过；既有真实视图 fixture 可编译。
- 历史 `labels_show_only_cn_us_and_timezone` 中 `region_label("JP") == "JP"` 的断言与本次新需求冲突，按约定保留原用例，不把该断言视为新需求的验收标准。未新增或修改测试用例。
- 尚待 06：实际 GUI 逐项中文显示、中文/小写代码搜索、保存/重开及未知旧地区可见值核对。此票不提前标记 resolved。

### 2026-10-16：修复搜索冻结并同步用户批准的旧断言

- 原实际 GUI 在地区搜索键入单字符 `j` 后变为未响应，3 秒 CPU 增量约 3.72 秒；仅展开和聚焦输入框不复现。窄日志定位为旧 `localize_tz_words` 在 `UTC+12` 上不断执行 `UTC → UTC` 恒等替换。地区共享搜索串使时区候选扩展到宿主全列表，从而触发这段既有死循环；HEAD 原实现已有该映射。
- 修复仅删除无意义的 `("UTC", "UTC")` 替换项，清除临时诊断日志，没有修改 overlay 或 vendor。
- 复用真实 Profile UI fixture 后，原 `jp` 操作 CPU 增量约 0.1875 秒 / 2 秒且保持响应，截图 `target/issue-1-region-debug/jp-fixed.png` 显示 `jp` 只命中“日本”；中文 `日本` 同样只命中“日本”（`chinese-sendkeys.png`）。fixture 构建通过。
- 用户随后明确授权仅将既有 JP 旧断言期望同步为“日本”，已只修改该条期望；未新增测试用例，其他原用例不变。`cargo test -p envbox-app --bin envbox-app`：62 通过、0 失败、5 忽略，既有地区标签/筛选检查全部通过。
- 使用独立 fixture PID、明确窗口 rect 与原生鼠标点击完成 JP 选择、保存和进程重启。保存 TOML 为 `region = "JP"`，Locale/UI Language 仍 `en-US`，Windows/IANA 仍 `Pacific Standard Time` / `America/Los_Angeles`。`target/issue-1-region-debug/jp-reopened-native.png` 显示重开后卡片、徽章及只读“地区”均为“日本”。先前点击脚本未完成选择，使用独立点击路径即可正常完成，无需改 overlay。所有本次自启诊断 fixture 已停止，主验收根配置未修改。

### 2026-10-16：补齐全部地区列表及未知旧地区 GUI 证据

- 在 `target/issue-1-region-debug/` 的独立 Profile fixture 展开地区列表并拖动列表滚动条，实际查看全部 11 个中文选项：顶部“中国、美国、台湾、新加坡、日本”，中段“新加坡、日本、菲律宾、英国、德国”，底部“英国、德国、韩国、玻利维亚、土耳其”。截图分别为 `regions-top-final.png`、`regions-middle-final.png`、`regions-bottom-final.png`，均位于该目录，已逐张查看。
- 在单独 `target/issue-1-region-unknown/` 中只将 Profile 的地区改为旧/外部未知代码 `ZZ`，不放入 Application。真实 fixture 加载成功并保持响应：卡片、徽章、只读“地区”显示 `ZZ`，进入编辑后已选值和下拉首条选项也显示原始 `ZZ`，没有默认为中国或隐藏。检查该独立 TOML 仍为 `region = "ZZ"`。
- 未知值截图为 `target/issue-1-region-debug/unknown-ZZ-readonly.png` 与 `unknown-ZZ-dropdown.png`，已查看。已有日本中文/小写代码搜索、保存重启、存储标识不变和 62 个原有 app 检查通过证据继续有效，未重复运行。
- 本轮未修改产品代码、未新增测试用例，未触碰主验收配置或管理/启动入口；自启 fixture 均已停止，桌面已交还。

### 完成结论

全部地区中文显示、中文/代码搜索、选择保存重开、未知旧值保留均有实际 GUI 证据；用户批准的单条历史断言同步后，62 个既有 app 检查通过。主 Agent 已查看并采纳最终截图，此票 resolved。
