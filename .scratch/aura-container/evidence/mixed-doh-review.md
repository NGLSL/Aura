# Mixed recovery / DoH 底座统一 code-review

Date: 2026-10-04
Baseline: `9919ad1200e2406f1059e5a577a4a965cdd57ab7`
Branch: `dev`
Status: reviewed-with-partial-acceptance

按 implement / code-review Skill，在上次交付后复核本轮工作树。Standards 与 Spec 由独立只读 Agent 并行审查，PKI 另有锁定源码与本机材料专项复核。审查不能替代未注入 Host 实际运行，也不把底座或规划当作完整 P0–P8。

## Standards

最终 **0 项未处理 actionable finding**。过程中修复：首次恢复剔除退出成员后 known_members/member_runtimes 不同步；safe Rust 可构造任意取消 callback/context；链容量超限被误报 Disallowed；默认 workspace 构建意外合并 fixture trust feature；Broker 停止漏唤醒。Owned handles、同步 C ABI、任务 abort/await、数量与字节上限及最后 deadline 阶段检查均已复核。

## Spec

最终 **0 项未处理 actionable finding**。恢复必须保存成员各自实际 Runtime 证据、重验完整 DTO 与 fresh challenge；schema 2 只作严格候选；未知 console companion 仍 Lost，未伪造 Verified。DoH 最后 exchange 入口检查避免初始化耗过预算后仍连接；真实 listener 回归锁住网络副作用。未放宽 PID/generation/token/remote 鉴权、未调 DNS timeout 或降低断言。

## 验证与发现的真实竞态

第一轮 workspace 卡在 DNS 集成测试，但 Probe 已退出，CLI 主线程等待自己的 Broker。对本次测试的唯一 owned pipe connect/close（发送 0 bytes）后，CLI 17 ms 正常退出；`target/broker-stop-race-proof/` 保留真实 generation、WCT、server PID 与干预时间。该轮最后 exit 0 **属于 assisted，不算无竞态通过**。

最小原生回归暂停下一 pipe 发布，确保真实 stop/nudge 已执行，再发布 pipe：旧代码 250 ms 内无法停止，RED 用例 0.26 秒；额外 nudge 仅在失败后清理测试线程。修复在 CreateNamedPipe 后、阻塞 ConnectNamedPipe 前重查 stop，使提前 nudge 由检查覆盖，之后 nudge 可连已发布 pipe；连接读写也检查 stop。屏障只属于 cfg(test) 的单个 Broker。Fresh Host 13432 Runtime=0，Broker 5/5、真实鉴权 7/7 GREEN。

最终未干预的源码全套 `cargo +1.99.0 test --locked --workspace --no-fail-fast`：**388 passed / 0 failed / 33 ignored**，exit 0，81.21 秒，fresh WMI Host 10452 Runtime modules=0。`target/workspace-mixed-doh-head.log` 与 result JSON；DNS 28 项正常结束。Ignored 不计通过；mixed/legacy native 7 例和 DoH snapshot 等有单独实际证据，其他未执行项仍保留。

`cargo +1.99.0 build --locked --workspace` 通过，最终日志 `target/workspace-mixed-doh-head-build.log`。DoH 修后最后真实 listener RED→GREEN 与双架构 native FFI 另行归档。Native Runtime 未改，测试明确使用既有冻结 V3 pair；DoH Rust staticlib 是独立原型产物，不能据此宣称 Aura Runtime 已链接它。

## 未完成的验收

- 票 12：OS console companion、NoJob/未知完整树终态、真实 OS reboot。四个 mixed GUI 组合与三个旧 schema 组合的通过不能推断所有树。
- 票 18/19：原生 ROOT/CRL 下公共 DoH 正向、IPv6 bootstrap、其他支持的 Windows 版本与完整系统观测；正式 router/Query Engine/actual Profile 注入/staging 接线。56 个 fixture 成功不能解除整体 No-Go；dns_doh=0 与 Core 拒绝仍保留。
- 安装升级、可见 GUI、驱动/WFP/强制文件与 Registry 后端、签名/VM 故障测试等完整 P0–P8 既有缺口。

具体运行与边界见 [mixed recovery](mixed-recovery.md)、[DoH 原型](doh-rustls-prototype.md)、[设计与原始依据](doh-rustls-design.md)。本轮交付是当前分支经过复核的实现与持久证据，无 push/release 或宿主驱动/信任库配置修改。
