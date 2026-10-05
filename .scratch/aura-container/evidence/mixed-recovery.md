# 混合架构 Runtime 成员恢复

Date: 2026-10-04
Related: [12 Supervisor 恢复](../issues/12-ticket.md)
Baseline: `9919ad1200e2406f1059e5a577a4a965cdd57ab7`

## 改动与身份契约

Run journal schema 3 新增逐成员 Runtime facts：PID、真实 creation generation、实际模块路径与 SHA256、完整配置摘要和 Runtime 版本。新增 Job 成员先通过当前 Broker 的完整身份认证，再原子持久化；Job membership 本身不能替代 Runtime identity。恢复按成员自己的模块身份重建 expectation，重新检查 Job、generation、实际已加载模块、完整 immutable DTO 与 fresh nonce，不猜测另一架构的 sibling DLL。

旧 schema 2 仍可读取。它的根 Runtime 模块只作为候选；每个存活成员必须实际加载相同候选路径/hash并通过完整重验，才能恢复。旧 mixed 记录没有可信逐成员证明时保持 TrackingLost，不从目录或架构推导新身份。

Review 找到并修复一次恢复后集合不同步的问题：已退出 root 被剔除时，known_members 与 member_runtimes 必须同时更新成最终重验的实际成员集合，然后原子写 journal。否则第二次恢复会被自身 schema 3 一致性校验拒绝。

## 本机实际矩阵

`target/mixed-gui-matrix.log`：**4 passed、0 failed**，19.29 秒；fresh WMI Host PID 19512，Runtime modules=0。

| root → child | root 状态 | 验证 |
| --- | --- | --- |
| x64 → x86 | root 存活 | 连续两次实际终止/重启 Supervisor，逐成员重新确认 |
| x64 → x86 | root 已退出，child 存活 | 连续两次恢复，第二次消费首次写出的新 journal |
| x86 → x64 | root 存活 | 连续两次恢复，保留对应 x86/x64 实际模块路径 |
| x86 → x64 | root 已退出，child 存活 | 连续两次恢复，存活 child 使用其 x64 模块身份 |

每例同时验证 A/B 的新确认、旧管理 generation 拒绝、Stop A 保留 B 与原生 Host、损坏 A 显示 Lost 而 B 可以继续新 Run/Stop。日志打印真实模块路径与成员 generation，而不是用进程数量推断恢复。

使用既有冻结 V3 Runtime pair，不覆盖前序证据：

```text
x64 215c42dc101265a45343c8f599acdd9de2aabe999578ecf60011f81c54146d36
x86 438d56e88e73eb0d2fca87400ec2f134c937041555c709611bdb2857be0f7e1a
```

复现入口 `crates/envbox-supervisor/tests/run-mixed.ps1` 构建独立原生 parent，再从未注入 Host 执行显式 ignored native recovery fixtures。普通 cargo test 中 ignored 不计通过；这里的四例是另行实际运行。

## 保留的失败与覆盖边界

旧 schema 2 的 x64 root → x86 child 在 `mixed-red` 上实际恢复为 Lost，根模块候选不匹配存活 child，作为行为 RED 保留。新原生 Console/混合夹具还暴露 OS console companion：实际 Job 包含 `conhost.exe`，它没有 Aura Runtime identity；真实目标 child 的入口 marker 已出现，因此不能把这次失败归因混合架构注入失败或 Detours helper 残留。

证据 `target/mixed-console-unknown.log`、`.stderr.log` 与 `mixed-images.stderr.log` 保留。已确认的实际成员例为 root PID 2352、x86 child 9260、`C:\Windows\System32\conhost.exe` PID 25148，child_entry_marker=true。父进程改为 GUI subsystem、child 指定 CREATE_NO_WINDOW 后也曾出现 companion，其完整创建来源尚未证明。

最终四例使用已有 GUI child，验证纯 GUI 应用树；没有过滤未知成员、延长身份超时或给 conhost 伪造 Runtime proof。Console/OS companion 仍是覆盖缺口。Live Supervisor 已持有的有效 Job 可按原作用域执行显式安全 Stop；崩溃后无法重新证明控制的成员不被认领，恢复失败的记录不能据旧 PID Stop。

`target/legacy-schema-green.log` 另行实际 **3 passed、0 failed**，9.91 秒，fresh WMI Host PID 8324、Runtime modules=0。旧 schema 2 同架构 root 单成员，以及 root 退出、同架构 child 存活，都通过连续两次崩溃恢复。旧 mixed 记录删除逐成员 proof 后明确 TrackingLost，公开 Stop 返回 NotControlled，目标仍存活；B 的 schema 3 正常恢复 Running，原生 Host 仍存活。

票 12 未整体关闭：真实 OS reboot、NoJob/未知完整树的终态及 OS companion 能力仍待实现或验收。最终全套检查另行补充，不从这些组合推断所有进程树可恢复。
