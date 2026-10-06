# DoH Standard TLS / Runtime 双轴审查

Date: 2026-10-06
Baseline: `a0dbac904865769915a16b501babe66d7f6e7362`
Scope: 当前 dev 工作区 diff 与新增文件；用户授权将 DNS strict 与 DoH TLS 吊销策略分离，修复真实 Profile DoH 数据面。

## Standards

独立 Standards review 未发现 actionable finding。复核包含固定 DER 根经本机 deny/restriction 过滤、Standard/StrictOffline 的 known revoked 行为、未知 FFI policy 的提前拒绝、caller-thread callback 与 RAII、literal bootstrap / 同一 deadline 与取消、CMake 架构和配置隔离、GUI/CLI/DTO/snapshot 完整传递，以及 HTTP/2 authority 修复。没有要求额外抽象的 Fowler smell。

最后 native decoder guard / regression probe 的增量审查未发现 hard finding。新增 map 回调借用/复制生命周期、Toolhelp handle 关闭、提前 strict 拒绝和 TLS policy 负向断言均符合契约。Reviewer 另提示 config-policy-probe 没有纳入 build.ps1/run.ps1 的自动 acceptance 执行：本轮作为独立 fresh WMI RED/GREEN 回归，README 和证据已明确该边界，未声称自动门禁已覆盖。

## Spec

独立 Spec review 发现一项 P1：原生 Runtime 共享 decoder 接受 VirtualView `strict=false`，与“不支持的非 strict 模式必须拒绝启动”要求不符，虽然 Rust launcher 已拒绝。新增直接链接实际 decoder 的 probe 先取得 RED，再加共享门控，双架构 GREEN。缺失/未知 TLS policy 同时验证拒绝，Host 非 strict 仍允许；修复详情和原始证据见 [实现证据](doh-standard-tls-runtime.md)。最终独立增量复核确认 P1 resolved：IPC 的 FillProfileFromMsg/DecodeDnsMessage、ENV 的 LoadFromEnvValues、EnvBoxLoadProfile 的 IPC→ENV 备用加载以及 DllMain 启动检查均经过校验成功的 Profile，未发现绕过共享 decoder 的路径；没有可执行的残留 finding。

公共 IPv4 DoH 的 `error=0` 表示成功。最终 native JSON 的三个分项 true、真实 Profile 16 项通过，与全局观测 `gate=false` 是不同的验收范围。早期报告“公共正向仍为 0”已经由 reviewer 核对原始 JSON 后纠正，不作为剩余失败。

IPv6、跨 Windows 版本、全局流量捕获、安装升级和 VM/驱动验证仍属 partial/unverified，未关闭完整 container 的 18/19 门禁。DoH 的实际当前 Windows IPv4 路径已取得正向证据。

最终 Rust 1.99 workspace build/test exit 0，418 passed / 0 failed / 34 ignored；最终 guard pair 的真实 Profile 16 项复验通过，独立 native decoder 双架构各七项通过。

Standards：0 项 hard/actionable 生产问题，自动执行 config probe 的验证覆盖边界已说明；Spec：1 项 P1 已修复并独立复核 resolved，0 项未解决 actionable finding。完整 Container 的未验证范围继续保留。
