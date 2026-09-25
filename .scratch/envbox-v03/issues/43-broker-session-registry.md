Parent: .scratch/envbox-v03/spec.md

# 43: envbox-broker 进程与 Session Registry（Phase 2a）

**What to build:** 独立 `envbox-broker` Host 进程：维护 Session Registry（PID → Session/Profile），提供 Runtime IPC 服务端；协议与现有 Named Pipe 消息契约保持兼容。

**Blocked by:** 42

**Status:** resolved

- [x] Broker 可作为独立进程启动；生命周期覆盖 Runtime 握手窗口（CLI/GUI 拉起或按需拉起）
- [x] Session Registry：创建/更新 EnvironmentSession，登记 root/child PID，解析 PID → Profile
- [x] Named Pipe 服务端接受 HELLO / GET_PROFILE，返回 PROFILE；忽略未知消息名与未知键（前向兼容）
- [x] 接收 PROCESS_CREATED / PROCESS_EXITED / RUNTIME_READY / HOOK_ERROR（可落 events 日志）
- [x] 协议契约测试（FakeBroker / 真 Broker 均可）：握手往返、错误与超时 Fail Open 不崩进程
- [x] 不在此票删除 ENVBOX_* 通道；不改变 Win32 运行语义

## Comments

### 2026-09-27 done

新增 crate `envbox-broker`（lib + `envbox-broker.exe`）。SessionTable 升级为 Session Registry（REGISTER_PROFILE / BIND_PID / 子进程继承 / exit 清理）。进程内 HostBroker 仍作过渡实现，协议同构。
