# 统一 code-review

Date: 2026-10-04
Baseline: `8a971ab9b768412f0e8bbddbfea65c2b1966c0fb`
Scope: baseline 后授权的 tracked/untracked 工作树，包含原有 DNS/Rust 1.99 改动及本轮独立 Container 切片。
Status: reviewed-with-partial-acceptance

按项目 code-review Skill，由独立只读 Agent 并行审查 Standards 与 Spec；行为证据由实现者/未注入 Host 测试提供，review 不冒充运行验收。完整 P0–P8 未完成，两个轴的 0 actionable finding 不表示 49 张票已通过。

## Standards

最终源码复核为 **0 项未处理 P1/P2**。过程中修复：Audit 后设置 LastError；child cleanup 只有真实 wait 确认死亡才上报 Exited；恢复 journal Runtime hash 使用本地普通文件检查、64 MiB 上限、流式读取与 deadline。另复核 staged DLL capability、完整身份 ACK、精确 generation 的退出后缓存、query-only Job escrow、本地 TLS chain 与明确 fallback。

## Spec

最终源码复核为 **0 项未处理 actionable finding**。修复旧兼容入口不等待实际 Runtime 身份、Packaged 确证已有 PID 的错误认领、安装包缺 Supervisor、旧 generation TrackingLost 被 GUI 隐藏和默认完整域名审计。Legacy 仍为 Compatibility；明确 gate 才有 EXE entry 前 release，导入 DLL 初始化与 EXE TLS 属于不同范围。

## 未完成的验收

- 12 mixed architecture 恢复、NoJob/未知完整树的终态、真实 OS reboot；不能由已知 PID 列表推断完整树已退出。
- 完整进程入口、真实 Chromium renderer、实际 Packaged 单实例/并发、不同用户/IL/remote。
- DoH 正式数据面与 bootstrap 隔离 Go；non-strict fallback；DoT 产品受信正向、辅助网络请求和同步 chain deadline 的全场景证据。
- 安装升级、长期资源/并发稳定性；WDK/VM/签名依赖的 driver、WFP、强制文件及 Registry 后端。

最后 workspace 首轮测试暴露 Git 工具输出回归，发生在以上源码 review 后。真实 RED 证明继承注入的 wrapper child 偶发缺少 sealed Runtime expectation，完整 IPC 后的新 mandatory ACK 因而拒绝初始化；Host 与 no-inherit 对照成功。修复使完整 IPC legacy parent 同样在 Resume 前 REGISTER_CHILD，保留 ENV fallback 的 Unverified 行为、原始挂起及 gate 权限。

两个 reviewer 已独立增量复核该 hunk，均无新增 actionable finding。相同 capture 场景由 RED 连续 20 次有 14 次空输出，变为 V3 32/32 精确 stdout + exit 0；没有 sleep 或降低断言。最终完整 suite 仍独立运行，不能以 review 通过替代行为检查。早期和最终运行证据见 [实施进度](implementation-progress.md)。

最终完整 suite 已通过：378 passed、0 failed、25 ignored，未注入 WMI Host、冻结 V3。其间 channel 并行测试的 artifact 冲突仅修复 fixture 端点互斥，内部四客户端仍并发，连续五轮及最终全套通过；未放宽生产认证/超时。native nongated child 六例亦通过。构建与行为结果支持本轮具体切片，不取消上列未完成验收。
