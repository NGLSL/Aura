# 无 VM 本轮增量统一审查

Date: 2026-10-06
Baseline: `ca50ed3`（当前工作树增量，包括新增文件）
Branch: `dev`
Spec: `.scratch/aura-container/spec.md`、`implementation-plan.md`、05/12/13/20/22/23 号票；IPv6 按用户明确决定延期。
Standards: `AGENTS.md`、领域/issue tracker 规范及 code-review Skill 的 smell baseline。

Standards、Spec 由独立只读 Agent 并行审查，WDM 引用/锁/身份边界另有独立 risk review。审查通过与实际运行资格分别记录。

## Standards

发现并修复六项：

- P1 bundle recovery 双重读取且缺共享期限：最终 stable lease 单次 hash，共享 cooperative deadline，本地绝对路径/reparse 检查；同步 OS I/O 的不可硬取消边界明确。
- P1 controlled 状态由可变环境读取：初始化锁存，所有受控判断/传播和 required Hook 检查复用同一状态，双架构真实 RED→GREEN。
- P2 repeat fixture 的 Profile 映射不完整：保存 Profile ID/name/expected status，旧结果明确映射，缺失明确失败。
- P2 C++ consumer linkage：协议头 extern C，真实双架构 C++ 链接检查。
- P2 Probe cleanup 未严格验证等待：仅 WAIT_OBJECT_0 成功，保存 WAIT_FAILED 错误、timeout 失败。
- P2 原始 Runtime path 与 canonical path 的身份竞态：先 pin 原路径，再对 canonical 实际 handle 核对 file ID，dedup 对照 lease identity；最终只 hash 原 stable handle。受控 seam 的 RED→GREEN 使用真实文件 ID/共享拒绝，不冒称实际并发竞赛。

最终第六项已独立增量复核解决，Standards 当前 0 actionable；修后 Supervisor 14/0、实际 mixed recovery 4/0，定向证据单独记录。

## Spec

发现并修复三项：

- P1 Job termination 失败被普通 refresh 清除：即时 TrackingLost 原子 journal、stop_failed 保持原因、失败 replay Partial；只有明确新 Stop 成功才清除失败并推进停止。
- P2 startup cleanup 忽略 journal 写失败：统一 persistence helper，返回错误/内存原因明确，真实锁 journal 不损坏旧 sealed record。
- P1 公共 DNS repeat 缺独立 provenance：每 attempt 前后验证实际 CLI/Probe/Runtime hash，检查 Runtime loaded、新审计；hash 漂移在 query 前拒绝且结果保留。

最终 Spec 增量独立复核确认以上三项修复，当前 0 actionable findings；完整规格的 Partial 项保持下述门槛。

完整 P0–P8 仍是部分实施。GUI 退出后 DNS、独立全局 capture、真实应用/特殊入口、OS reboot、完整 conhost、可信系统服务/WFP/文件/Registry 实际后端、驱动加载/Verifier/安装兼容性资格仍缺，不能关闭全部票。WithToken 是实际 Unsupported 拒绝而非正向传播支持。IPv6 不作为当前阻塞项。

## WDM 专项风险

文档 P2：process callback 的 child 拒绝说明过宽。README/ADAPTER 已限定实际触发通知路径，Native clone/PSS VA clone 是独立强制资格门槛；未证明可信归属或实际拒绝前禁止加载或启用 Container/Strong。独立复核已确认该文档问题解决，无新增可操作源码 finding。该结论不等于克隆机制或真实内核运行已验收。

## 验证入口

本轮实际结果、冻结 DLL hash、历史失败及最终定向检查见 [跟进证据](nonvm-completion-followup.md)。SYS 是 source-only prototype；没有加载、安装服务、改变系统信任或网络。未打包、未推送或发布。

审查结果：Standards 0 未处理 actionable（本轮六项已修复）；Spec 0 未处理 actionable（本轮三项已修复）；WDM 专项文档一项已修复。各轴结论独立，不将明确未交付的总规格资格转为已完成。
