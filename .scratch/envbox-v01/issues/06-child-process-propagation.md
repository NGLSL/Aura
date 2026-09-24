Parent: .scratch/envbox-v01/spec.md

# 06: 子进程传播（Process Tree Instance）

**What to build:** 目标进程树内创建的 `cmd`/`node`/`powershell`/`git`/`python` 等子进程默认继承同一 Environment Profile；用户从任务栏另开的进程完全不受影响。隔离单位是 Process Tree Instance，不是可执行文件名。

**Blocked by:** 05 Minimal Profile — 4 个核心 API

**Status:** ready-for-agent

- [ ] Hook `CreateProcessW`/`CreateProcessA`：强制 `CREATE_SUSPENDED` → 注入 Runtime → 保证 Profile ID 继承 → 仅当 `caller_requested_suspended=false` 时 Resume
- [ ] 调用方已要求 suspend 时注入后保持 Suspended 并返回调用方
- [ ] `envbox-probe --spawn-child` 父子虚拟化字段一致
- [ ] 矩阵：`cmd`→`node` 树（及 git/powershell/python）继承；无关进程不受影响
- [ ] 同树共享同一 `ENVBOX_INSTANCE_ID` / Profile 视图
