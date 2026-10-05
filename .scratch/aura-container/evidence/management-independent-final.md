# Ticket 09 same-user integrity boundary evidence

日期：2026-10-06

范围：当前 Windows 主机、同一用户 SID、低完整性客户端直接访问 medium Supervisor endpoint。

状态：本机真实负向通过；不同用户、远程客户端和 high Supervisor 的独立环境矩阵仍未声称覆盖。

## 验收目标

这次验证专门排除一种容易产生假阳性的测试方式：低完整性客户端不能调用生产
`SupervisorClient::endpoint()` 后，把自己的低完整性 endpoint 的 `NotFound` 当作
“服务端认证通过”。测试先由当前 medium/high 管理进程启动 Supervisor，读取它实际
生成的 endpoint，再把这个 endpoint 原样交给低完整性 probe。probe 不使用 Aura 的
endpoint 推导逻辑，直接调用 `CreateFileW` 连接该 medium/high endpoint。

如果 Windows 对该 pipe 的 DACL 或 Mandatory Integrity Control 在连接阶段拒绝，测试
记录实际 Win32 error；如果将来的 pipe 安全描述符允许 low IL 打开，则 probe 继续
发送真实 `Ping`，并要求服务端返回 `AuthenticationDenied`。无论哪条路径，原 medium
manager 都必须用原 generation 再次 `Ping`，状态为 `Ok` 且 generation 不变。

## 实现和安全边界

`tools/envbox-management-fixture/main.cpp` 使用当前进程的 same-user token：

1. `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)` 创建受限 primary token。
2. 使用 `SetTokenInformation(TokenIntegrityLevel)` 设置
   `S-1-16-4096`（low integrity）。
3. `CreateProcessAsUserW` 启动真实 probe 子进程。
4. 通过继承的匿名 pipe 收集 probe 结果，避免要求 low IL 进程向普通 medium
   `target` 目录写文件。
5. helper 对自己的子进程设置 10 秒等待上限；超时只终止本次创建的子进程，随后
   关闭其 process/thread/pipe/token handles。

本测试不调用管理员命令、不设置 pipe ACL、不修改注册表、不修改宿主系统安全策略。
低进程报告自己的 SID 和 integrity RID，父测试要求 SID 与 launcher 相同、RID 为
`0x1000`，并要求 launcher/server owner 至少是 medium (`0x2000`)。

## 实际结果

入口：

```powershell
./tools/envbox-management-fixture/run.ps1
```

Fresh host 记录：`target/management-independent-final-host.log`

```text
PID=5228
RuntimeModules=0
Architecture=x64
```

测试记录：`target/management-independent-final.log`

```text
launcher_integrity=8192
create_process=ok
child_exit=0
probe_integrity=4096
probe_sid=S-1-5-21-3398544156-641326181-3609602823-1001
same_sid=true
target_endpoint=<the exact medium endpoint passed to the probe>
pipe_open=denied
pipe_open_error_code=5
pipe_open_error=5
low_integrity_management owner_integrity=0x2000 low_integrity=0x1000 same_sid=true direct_endpoint=true manager_ping=ok generation_preserved=true
test result: ok. 1 passed; 0 failed
control_missing_exit=29
control_missing_pipe_open=error
control_missing_pipe_open_error_code=2
```

本次 x64 native fixture artifact SHA256：
`480645C5E1EF3C3CF375DC080C74980469858BEE80499ACE1392A906823E4D04`。

不存在 endpoint 的控制样本也由同一个 runner 执行：helper exit 为 `29`，probe
记录 `pipe_open=error`、`pipe_open_error_code=2` (`ERROR_FILE_NOT_FOUND`)；runner
只有在这个非零结果出现时才成功结束。由此可见 `NotFound` 不能伪装成预期的
`ERROR_ACCESS_DENIED` 负向。

这里的 `pipe_open_error=5` 是 `ERROR_ACCESS_DENIED`。这台机器的默认 named-pipe
安全检查在请求抵达 Supervisor 前就拒绝了 low IL 对 medium pipe 的直接打开；因此
本次不能把结果描述成服务端收到了 `AuthenticationDenied`，而是明确记录为 OS
pipe ACL/MIC 负向。测试仍证明了 low IL probe 真实存在、same SID、使用了正确的
medium endpoint，并且随后正常 manager 可以继续使用同一个 generation。

## Microsoft 依据

- [Named Pipe Security and Access Rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)：`CreateNamedPipe` 使用
  `NULL` security descriptor 时采用默认 descriptor；客户端 `CreateFile` 连接
  named pipe 时执行访问检查。
- [Mandatory Integrity Control](https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control)：MIC 在 DACL 之前评估；默认对象策略为
  `SYSTEM_MANDATORY_LABEL_NO_WRITE_UP`，low integrity 进程不能向 medium 对象写入。
- [CreateRestrictedToken](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-createrestrictedtoken)：可以从调用者自己的
  token 创建受限 token；受限自身 token 用于 `CreateProcessAsUser` 时不需要
  `SE_ASSIGNPRIMARYTOKEN_NAME`。
- [CreateProcessAsUserW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw)：新进程运行在指定 token
  的安全上下文，并说明 token 权限和句柄继承要求。

## 未覆盖项

本证据只覆盖同 SID 的 low → 当前 medium/high owner，且本机看到的是连接阶段
`ERROR_ACCESS_DENIED`。它不伪造其他用户账户、不把远程 pipe 当作本地测试，也没有
在无 elevation 的当前会话中创建 high Supervisor。后续若需要证明服务端逻辑而非
OS 前置拒绝，应在隔离环境建立明确允许 low IL 读取/写入的测试 pipe ACL（只限测试
fixture，不能修改生产 endpoint），然后复用同一个直接 endpoint probe 验证
`AuthenticationDenied`。
