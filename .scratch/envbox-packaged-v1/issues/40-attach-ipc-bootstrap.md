Parent: .scratch/envbox-packaged-v1/spec.md

# 40: Runtime Attach + IPC Bootstrap

**What to build:** 共享 RuntimeInjector；AttachStrategy PreExecution/PostActivation；Runtime 经 Named Pipe 按 PID 取 RuntimeProfile（Win32 环境变量回退保留）。

**Blocked by:** 37, 38

**Status:** ready-for-agent

- [ ] `RuntimeInjector::attach(pid, dll)` 双后端共用
- [ ] PreExecution：inject → handshake → resume；PostActivation：probe supported 后 inject
- [ ] IPC：HELLO / GET_PROFILE / RUNTIME_READY / HOOK_ERROR（可扩 PROCESS_*）
- [ ] Win32 在 IPC 不可用时仍可经 ENVBOX_* 启动（回归）
- [ ] mediumIL supported：Runtime 加载 + handshake + Probe 见 Profile
- [ ] 注入失败 = Startup Fail Policy，禁止静默无虚拟化

## Comments

**限制（写入 README）：** Packaged root 为 PostActivation，存在 early-start race；子进程仍 PreExecution。
