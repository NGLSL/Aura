# 无需 VM 切片的统一 code-review

Date: 2026-10-06
Baseline: `a5baed8`
Branch: `dev`
Scope: baseline 之后本轮授权的 tracked/untracked 工作树；DNS 异步生命周期、x86 Probe/capability、恢复/bundle lease、独立管理/入口/DoT/DoH/空驱动验收及证据。
Status: reviewed-with-partial-acceptance

按项目 code-review Skill，由两名独立只读 Agent 分别审查 Standards 与 Spec。固定点为开始本轮工作时的提交；实施中的未提交文件纳入审查。规范来源为项目 AGENTS.md 与 docs/agents，规格来源为 `.scratch/aura-container/spec.md`、implementation-plan 与相关票，以及 DNS transports 规格。用户明确授权全部 P0–P8 的后续推进，旧 V0.1 禁止提前做驱动的范围规则不阻止本轮空驱动构建切片。

## Standards

最终增量复核：**0 个未处理 actionable finding**。

审查中修复 Runtime bundle 文件路径/hash 的 TOCTOU：稳定只读/read-share handle、Windows file identity、64 MiB 有界 handle hash 与已认证摘要匹配。管理负向 fixture 仅将 OS error 5 算作拒绝，不存在 endpoint/error 2 为失败。DNS cancellation token 读写统一加锁，完成后不再回写 caller storage；ExW 最后原子发布终态，并按原生状态读取 helper，不强行 Hook 短 prologue。

DoH harness 的五项修复包括唯一结果和 bounded WMI wait、TCP/TLS 预算、unexpected acceptance 非零、完整 DNS question 校验与 generated trust variant 的唯一源码替换。最后又修复外部重复 RunId 的旧结果复用：顶层拒绝外传 RunId，worker 校验 GUID，已有结果拒绝。real-app JSON 使用同目录临时文件及原子 Move/Replace，父端解析失败 bounded retry；实际 tiny fixture 覆盖首次写入/替换/读取及清理。

审查结论来自冻结源码和各项必要验证；不是公共 DoH 正向、全系统网络观测或 driver 加载的证明。C# chain capture 的 timeout 只提供有界观察，不能替代生产 transport 的完整取消/销毁语义。

## Spec

最终增量复核：**0 个未处理 actionable finding**。

确认 ExW 提交 997、pending Internal 10036、borrowed result/Pointer/InternalHigh 写完后原子发布 terminal；callback/event 引用、event duplicate、callback 释放内存、取消单次完成、80 次无需 helper 的退休与重复 native helper 读取均有对应契约和 Probe。ExA async、不支持的 namespace/provider/flags 在 Profile 路径明确拒绝。DnsQueryEx callback 内旧 token 与重入后的 stale generation 均本地返回 87，不能取消新请求或转交 Windows provider。

修复真实入口矩阵的假 Verified、误将两侧 Chrome exit 13 归因 Aura、wrapper 退出码和按 PID 停止竞态；generation 核对、TerminateProcess 和退出确认使用同一 owned handle。最终审查发现 callback-free Probe 自身仍有释放后读取：已去掉提交后的 caller-state 读取，并改即时 fixture，保留其他模式的 pending 状态断言；reviewer 增量复核确认关闭该 finding。

Runtime 行为缺陷修复与完整验收门禁分开：本轮切片通过不表示 49 票全部完成。

## 独立运行证据与剩余门禁

最终 fresh WMI Host PID 19324、Runtime modules=0，Rust 1.99 workspace build exit 0；完整 suite **395 passed / 0 failed / 34 ignored**、exit 0。最终 x86 mixed 路径在另一个 fresh WMI Host PID 11012 验证 **6 passed / 0 failed**。冻结 DLL hash、原始日志和首轮/并行占用失败来源见 [实施进度](implementation-progress.md) 与 [Resolver](resolver-async-final.md)。

- DoT：最终 v2 双架构、每架构一次的 strict IPv4 公共注入 smoke 通过，不宣称全系统无辅助流量。
- DoH：本地双架构 IPv4 58 个场景通过；用户关闭 IPv6，10 个本地 IPv6 场景未执行。公共原生默认 verifier 为 UnknownIssuer，研究 AuthRoot variant 暴露 UnknownRevocationStatus；18 仍 No-Go，19 不启用。
- 入口与管理：部分真实入口及 same-SID low IL 的 OS 拒绝通过；renderer、WithToken、Native、Packaged、其他用户/remote 等未验证。
- 恢复与存储：稳定 bundle lease、TrackingLost journal 与 NoJob 保守终态通过；完整 conhost/跨架构恢复、真实 OS reboot、升级和 Supervisor 崩溃后的外部 cleanup 仍未完成。
- 驱动：真实 WDK/SDK 空功能 x64 SYS、INF/CAT 结构验证通过，unsigned；未安装/加载，VM、测试信任、Verifier、WFP/文件/Registry 强制后端、多 Windows 版本与 P4–P8 故障恢复仍待实施/验收。

总计：Standards 0 个未处理 actionable；Spec 0 个未处理 actionable。上述明确的缺失实现/验收仍保留，不由源码 review 或忽略测试升级为通过。
