Parent: .scratch/envbox-packaged-v1/spec.md

# 41: Session 归属、子进程统一与 Packaged V1 验收

**What to build:** 子进程传播统一；`belongs_to_session`（ancestry + package window）；IsolationGuarantee 落库；完整 V1 验收矩阵。

**Blocked by:** 39, 40

**Status:** ready-for-agent

- [ ] ChildPropagationManager：CreateProcess* 挂起 → register → attach → resume（不分 Win32/Packaged）
- [ ] 归属：root descendant **或** PackageFamilyName 匹配 + activation 窗口创建时间
- [ ] 持久化：package_family_name / aumid / isolation_guarantee / attach_strategy
- [ ] 实例停止后 package 配置/lifecycle 无污染
- [ ] 验收包（真实 mediumIL 应用）：AUMID→PID→probe→Runtime→IPC→Geo/Locale/TZ→Host 不变→子进程继承→stop 干净
- [ ] 文档：Tier 1/2/3 与 early-start race 限制

## Comments
