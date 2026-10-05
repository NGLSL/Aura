# GetAddrInfoEx async resolver final evidence

日期：2026-10-06
范围：ticket 16，Windows Runtime 的 `GetAddrInfoExW` event/callback/cancel 生命周期，以及 `GetAddrInfoExA/W` Profile 路径的明确能力边界。

## 实现结果

- VirtualView 只为文档支持的异步 `GetAddrInfoExW` 建立 Runtime-owned worker。Profile 查询走现有 Profile DNS wire transport；不会把失败请求回退到 Windows Host DNS。
- `GetAddrInfoExA` 的异步参数在 VirtualView 明确返回 `WSAEOPNOTSUPP`（10045）。Host 模式保持原生 API 行为。Microsoft 文档把 ExA 的异步参数列为保留参数，因此没有伪造 ANSI async 语义。
- W async 只接受 `nlp_id == NULL` 和 `NS_ALL`/`NS_DNS` namespace；其他 namespace/provider 在进入原生 provider 前返回 10045。结果指针、OVERLAPPED 和 callback/event 组合按照 Windows contract 校验。
- Profile 路径统一允许 `AI_PASSIVE | AI_CANONNAME | AI_NUMERICHOST`，对 `AI_ADDRCONFIG`、IDN、`AI_V4MAPPED`/`AI_ALL` 等未实现语义返回 10045；Host 模式不改变。
- 每个异步操作使用 map 引用和 worker 引用；完成时在同一锁内写入 caller result、`OVERLAPPED.Pointer` 和 `InternalHigh`，最后以原子写发布 terminal `Internal`。提交返回值仍是 `WSA_IO_PENDING`（997），pending 的 `Internal` 是 `WSAEINPROGRESS`（10036）；完成通知之后不再写 caller storage。
- `lpNameHandle` 只在提交同步写入 opaque token；context 不保存该 output pointer，也不会在 callback/event 完成后回写它。完成后的 stale token 由 map 判定为无效。
- event 模式接收时 `DuplicateHandle` 保存同对象的 Runtime-owned event，调用方可以在 worker 完成前关闭原始 event。event 完成后 worker 立即 retire map entry，即使调用方不调用 `GetAddrInfoExOverlappedResult` 也不会耗尽 64 项 pending 上限。
- callback 模式要求 `OVERLAPPED.hEvent == NULL`；callback 返回后只访问自有 context 引用，不再访问 caller 的 OVERLAPPED/result/name-handle。取消在完成竞态下至多产生一次 completion，并返回 `WSA_E_CANCELLED`（10111）。
- `GetAddrInfoExOverlappedResult` 保持 Windows 的 terminal status 幂等语义；重复调用不会被伪造成一次性消费。当前 Windows x64 导出函数 prologue 太短，未安装 Detours trampoline；worker 直接按文档发布 `OVERLAPPED.Internal/InternalHigh/Pointer`（`InternalHigh == 0`），原生 helper 只读并返回相同 terminal status。
- DnsQueryEx 完成时在锁内撤销旧 token 的写入 ownership，并保留不可转发的 opaque tombstone；callback 内或并发使用同一 `PDNS_QUERY_CANCEL` storage 时，旧 token 在本地判为 stale，新重入查询发布新 generation，旧完成不会覆盖新 token，也不会落到 Windows provider。

## Microsoft contract 依据

- [GetAddrInfoExW](https://learn.microsoft.com/en-us/windows/win32/api/ws2tcpip/nf-ws2tcpip-getaddrinfoexw)：W-only asynchronous support, callback/event exclusivity, manual-reset event, OVERLAPPED completion, and `lpHandle` lifetime.
- [GetAddrInfoExCancel](https://learn.microsoft.com/en-us/windows/win32/api/ws2tcpip/nf-ws2tcpip-getaddrinfoexcancel)：cancel handle contract.
- [GetAddrInfoExOverlappedResult](https://learn.microsoft.com/en-us/windows/win32/api/ws2tcpip/nf-ws2tcpip-getaddrinfoexoverlappedresult)：terminal result status and pending/invalid behavior.

## Verified

### Runtime builds

Both independent MSVC Release builds completed after all source changes:

```text
D:\Project\Aura\target\nonvm-async-runtime64\Release\envbox-runtime64.dll
D:\Project\Aura\target\nonvm-async-runtime32\Release\envbox-runtime32.dll
```

The source-freeze v2 pair used by the parent runner is copied to:

```text
D:\Project\Aura\target\nonvm-final-runtime-v2\envbox-runtime64.dll
D:\Project\Aura\target\nonvm-final-runtime-v2\envbox-runtime32.dll
```

SHA-256 of the frozen pair:

```text
envbox-runtime64.dll  CA8283ADAE000DBEAAE65902A10F2E0E05B94C38C14F7B3652EDD3066C43D965
envbox-runtime32.dll  5D2FD1038748F0D579F5B2EB59EBAEECDFF6C7846D12FD93876875696B6CEBE7
```

### Rust and CLI checks

The probe was checked and built with Rust 1.99:

```text
cargo +1.99.0 check -p envbox-probe --locked  -> pass
cargo +1.99.0 build -p envbox-probe --locked  -> pass
cargo +1.99.0 check -p envbox-launcher --locked -> pass
cargo +1.99.0 build --target i686-pc-windows-msvc --target-dir target/x86-launcher-v2 -p envbox-cli -p envbox-probe --locked -> pass
cargo +1.99.0 build --target i686-pc-windows-msvc --target-dir target/x86-probe-v3 -p envbox-probe --locked -> pass
```

The complete DNS CLI acceptance test passed with the v2 x64 DLL and the rebuilt Probe:

```text
cargo +1.99.0 test -p envbox-cli --test cli_dns -- --test-threads=1
32 passed, 0 failed, 77.84s
```

The test process used `ENVBOX_TEST_RUNTIME_DLL=D:\Project\Aura\target\nonvm-final-runtime-v2\envbox-runtime64.dll` and `ENVBOX_TEST_PROBE_EXE=D:\Project\Aura\target\debug\envbox-probe.exe` with the in-process fixture on UDP/15353.

This includes Profile routing, arbitrary QTYPEs, Host/VirtualView contrast, DnsQueryEx sync/async, callback re-entry with reused cancel storage, cancellation, unsupported ExA/ExW provider and namespace inputs, synchronous unsupported flags for ordinary A/W and ExA/W, event retirement beyond 64 pending operations, callback-side caller-storage release, and host DNS regression checks. The test fixture is an in-process Rust fixture bound to the configured loopback port and is stopped by the test process.

The final workspace runner also completed with `build_exit=0` and
`test_exit=0`; its aggregate was 395 passed, 0 failed, and 34 ignored. The
fresh Host observation recorded `host_pid=13116` and `runtime_modules=0`.
Raw runner artifacts are `target/workspace-nonvm-final-build.log`,
`target/workspace-nonvm-final-test.log`, and
`target/workspace-nonvm-final-result.json`.

### Direct x64 integration observations

With a corrected loopback UDP fixture returning A `10.99.0.1`, the frozen x64 pair produced:

```text
--dns-strict ex-w event
status=997, event signaled, first GetAddrInfoExOverlappedResult=0,
second GetAddrInfoExOverlappedResult=0, records=1

--dns-strict ex-w callback
status=997, callback status=0, callbacks=1, records=1

--dns-strict ex-w cancel
status=997, cancel=0, callback status=10111, callbacks=1, records=0

--dns-strict ex-w repeat
completed=80 (event mode did not call GetAddrInfoExOverlappedResult)

--dns-strict ex-w callback-free
status=997, callback status=0, callbacks=1
```

### Direct x86 mixed integration observations

The x86 acceptance used the x64 CLI launcher only for process creation and
injected the independently built i686 Probe. This avoids treating a 32-bit
CLI build as a substitute for the actual cross-architecture injection path:

```text
runtime: D:\Project\Aura\target\nonvm-final-runtime-v2\envbox-runtime32.dll
runtime SHA-256: 5D2FD1038748F0D579F5B2EB59EBAEECDFF6C7846D12FD93876875696B6CEBE7
probe: D:\Project\Aura\target\x86-probe-v3\i686-pc-windows-msvc\debug\envbox-probe.exe
probe SHA-256: 4236CF3650FC6DA08B3E487E07233AF29FE5D5FF3F5377EAB53DBBD8E9EF5377
DNS fixture: per-test loopback fixture on UDP/15354
```

The targeted mixed run passed with no failed tests:

```text
strict_getaddrinfoexw: 4 passed, 0 failed, 8.91s
dnsquery_ex_async:    2 passed, 0 failed, 4.05s
```

The four ExW tests cover event, callback, callback-free caller-storage
release, cancellation, and the 80-operation event retirement loop. The two
DnsQueryEx tests cover callback cancellation returning `ERROR_INVALID_PARAMETER`
(`87`) for the completed tombstone and stale pre-reentry generation, while the
new generation remains independent. The raw test output is
`target/resolver-async-x86-cli-targeted.log`.

Fresh process-level raw observations are also retained:

```text
Host (no Runtime injection):
  target/resolver-async-x86-host.stdout.log
  target/resolver-async-x86-host.meta.json
  pid=22888, exit=0, runtime_modules_observed=[]

Injected x86 Profile run:
  target/resolver-async-x86-injected.stdout.log
  target/resolver-async-x86-injected.meta.json
  launcher_pid=25296, probe_pid=17200, exit=0
  stdout marker: EnvBox Runtime Loaded
```

The injected run returned `StrictProbe_Status=997`, terminal result `0`, and
one record from the 15354 Profile fixture. The host run returned the same
native event completion shape without the Runtime-loaded marker. The v2 x86
DLL and v3 Probe hashes above identify the exact artifacts used by these
observations.

### Final review correction and fresh mixed run

Spec review found that the callback-free Probe read `OVERLAPPED.Internal`
after the callback could already have freed its enclosing allocation. The
Probe now performs no caller-state read after submission in this mode. It
uses only the separately owned observation; the fixture responds immediately
rather than delaying to conceal the race. Initial pending `10036` remains
asserted by the event, ordinary callback and cancel tests.

After this correction, both Probes were rebuilt. The final i686 Probe at the
same `target/x86-probe-v3/.../envbox-probe.exe` path has SHA-256
`6433AD536E471FF6CEDD85CF23BE181B3DD41B4068E504ADB7F0896EF2A226FF`;
the earlier `4236CF...` artifact and its observations above are historical.
Fresh WMI controller PID **11012**, with `Runtime modules=0` and inherited
`ENVBOX_*` cleared, ran the four ExW and two DnsQueryEx tests against the
unchanged frozen v2 x86 DLL and UDP/15354. Both commands exited 0: **6 passed,
0 failed**. Raw output and identity are retained in
`target/resolver-x86-fresh-final-exw.log`,
`target/resolver-x86-fresh-final-dnsquery.log` and
`target/resolver-x86-fresh-final-result.json`.

The older process-level metadata's empty/null module sample is an observation,
not independent live-DLL provenance proof. The mixed tests require the
launcher's authenticated Runtime identity and Profile results; separate public
DoT evidence captures actual live module paths/hashes for this same v2 pair.

VirtualView synchronous `ex-a sync-flags` and `ex-w sync-flags` both returned 10045 with no callback/event/token. ExA async, unsupported namespace and unsupported provider also returned 10045 before creating pending work. A fresh Host profile with the same frozen x64 DLL loaded Runtime and completed native ExW event resolution, preserving native Host behavior.

## Partial / unverified

- No Windows VM, driver, WFP, or cross-architecture fault-recovery environment was available. Those checks remain outside this evidence.
- The environment has IPv6 disabled; the mixed IPv4/IPv6 source/test paths are present, but live IPv6 transport behavior is not claimed here.
- Final workspace build/test evidence is maintained by the parent in [implementation progress](implementation-progress.md); package-level checks here do not stand in for that whole-suite result.
- The acceptance fixture proves Profile transport and the caller-visible lifecycle. It does not prove that unrelated applications using their own DoH/DoT/DoQ stack are intercepted; those paths require the separate transport/WFP work tracked by the parent task.

The independent native reference for repeated `GetAddrInfoExOverlappedResult` reads is recorded at `target/addrinfoex-native-reference.log`; it returned the same terminal status on both reads for fresh Host and native control runs. The reference is read-only ABI evidence and does not imply that Runtime installed a helper Detour.

Artifact identity is explicit: the v2 DLL pair above is the Runtime under test; the native reference log uses its separately recorded control executable (`exeSHA918C262...69AC9B2`) and is not a copy of either Runtime DLL. Earlier `target/nonvm-final-runtime-async` hashes (`F0F97...`/`6187...`) are superseded by the v2 pair and are not acceptance artifacts. The existing cross-architecture Host proof at `target/dns-strict32-green-result.json` records `host_runtime_modules: 0`, `exit: 0`, and its older `container-typed16-runtime` SHA; it is retained as historical raw evidence and must not be read as a v2 injection result.
