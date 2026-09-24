Parent: .scratch/envbox-v01/spec.md

# 10: GUI — Application 管理 + Run/Stop

**What to build:** 用户在 GUI 中 Add/Edit/Delete Application，一键 Run 产生 RuntimeInstance，Stop 结束整棵 Process Tree Instance，并能看到状态与子进程数量。GUI 不包含注入逻辑。

**Blocked by:** 06 子进程传播（Process Tree Instance）

**Status:** ready-for-agent

- [ ] Application 字段：Name、Launch type（Executable/Command）、路径/命令、Arguments、Working Directory、Default Profile、子进程继承开关
- [ ] Save / Run / Delete 可用；配置写入 storage
- [ ] Run 创建 RuntimeInstance；状态 Starting/Running/Stopping/Exited/Failed
- [ ] Root 退出但 Job 内仍有子进程时保持 Running
- [ ] Stop 关闭 Job（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 语义）并更新状态
- [ ] GUI 仅调用 core/launcher
