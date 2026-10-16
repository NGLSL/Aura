# 446a7f4 整体审查

Date: 2026-10-16
Base: `fab51c045920e095583d31935281cc435b6dd354`
Candidate: `446a7f4`
Diff: `git diff fab51c045920e095583d31935281cc435b6dd354...446a7f4`

## Standards

1 项 P2：`chrome::configure` 在独立目录关闭、检测意图存在且无显式目录时，固定使用 Stable 的默认数据根。应用识别接受其他渠道的 chrome.exe；Beta/Canary 等实际默认根不同，可能误报 Stable 占用，或启动后无法确认 singleton 而终止本次 Job。

## Spec

1 项 P2，同一缺陷违反“开关关闭时保留原有目录/参数语义”和“按实际浏览器目录确认归属”。依据 [Chromium Windows 数据目录定义](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md#Windows)。未发现其他确认可达的规格缺陷。

## 修复

共享 Chrome 启动模块现读取文件版本资源识别品牌，并按标准安装渠道确定各自数据根；无法确认的自定义布局要求用户显式指定目录或启用独立目录，不默认为 Stable。关闭开关的普通 Run 仍保持原 early-return 行为，没有额外前置条件。代码检查通过；本机没有其他渠道的二进制，未将静态核对写成 Beta/Canary 实际运行证据。

用户在审查中新增的“实际出口只读 + Profile HTTP/SOCKS5 代理”属于后续需求，按票 07 实施，不计作 446a7f4 的既有缺陷。

## 07 增量审查与运行闭环

新增代理链路的 Standards 审查发现 1 项目录 lease/handoff 缺陷，Spec 审查发现该缺陷和 `--` 后代理参数被忽略的缺陷；终止符带空白的同类残余也一起修复。实际运行又定位并修复严格快照未允许合法 proxy 字段的问题。修复后的受限 handoff、统一参数解析和快照校验经定向复核，无剩余可行动 finding；两种代理、两条启动路径与检测复用实际结果见[代理验收](./profile-proxy-acceptance.md)。
