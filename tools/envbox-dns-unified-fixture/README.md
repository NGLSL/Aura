# IPv4 DNS transport 注入矩阵

运行真实 CLI → Profile → Launcher → Runtime → Windows DNS API，不导入证书、不修改宿主 DNS、IPv6、代理或 trust。IPv6 网络连接按当前范围延期；AAAA QTYPE 仍在矩阵内。

```powershell
python tools/envbox-dns-unified-fixture/run.py `
  --cli target/doh-integrated-clients/debug/envbox.exe `
  --probe64 target/doh-integrated-clients/debug/envbox-probe.exe `
  --probe32 target/doh-integrated-clients/i686-pc-windows-msvc/debug/envbox-probe.exe `
  --runtime-dir target/doh-integrated-runtime-final-guard
```

建议从清除继承 `ENVBOX_*` 的全新 WMI 控制进程运行，归档该控制进程的 modules 检查和退出码。脚本自身也检查 Python controller 没有载入 Runtime，并记录每个 CLI、Probe、DLL 的路径与 SHA-256。冻结 Runtime 目录只读使用，不覆盖。

每次生成独立 `target/dns-unified-<uuid>/`，保留完整配置、审计和 `result.json`。异常或断言失败的部分记录仍保存，退出码非零不能解释为通过。没有删除临时数据或其他进程的操作。

覆盖内容：

- UDP/TCP 的本地受控报文：A、AAAA、HTTPS、SVCB、TXT、PTR、SRV、CNAME、NS、未知 65280，以及根名 NS，双架构各 A/W/UTF8/Ex/async 五入口。逐项检查 request wire、返回状态、记录类型/数量、Probe native free 及 Runtime 载入标记。
- TCP 连接失败后显式 UDP 回退、全部失败、异步取消、共享 deadline。
- 公共 Cloudflare DoT 和 DoH 的同一 QTYPE/API 矩阵。DoT 使用当前 Windows 严格信任策略；DoH 使用显式 standard 策略。公共服务可能正常返回 NODATA/NXDOMAIN，因此状态 0/9501/9003 均是完成解析，TLS/网络失败不能通过。
- Profile 审计的 `dns-host` 条目检查；这只是产品审计，不是独立网络抓包。

`--local-only` 只运行受控 UDP/TCP 和负向场景，公共 TLS 服务证据仍需单独执行。此选项不能用于宣称四传输全部完成。

上述 `run.py` 没有覆盖 GUI 退出、父子进程、其他 resolver 入口、并发长跑或独立 Host DNS/API trap；不能仅凭该矩阵关闭 20 号票全部验收，也不能声明应用自带 DNS/DoH 被阻断。AAAA 当前 Probe 描述为 opaque，矩阵检查其 native 记录存在与类型，而不宣称独立验证每个 IPv6 地址字节。

## 失败的独立复验

```powershell
python tools/envbox-dns-unified-fixture/repeat_failed.py target/dns-unified-<uuid>/result.json --repetitions 3
```

复制 Profile 到新目录，保存独立复验结果。新矩阵直接记录 Profile ID/name；旧矩阵使用明确的 transport→Profile 名字映射，无法匹配时明确报错。每次 attempt 前后重新计算 CLI、Probe、Runtime 的路径/hash，必须匹配原冻结证据；任何产物漂移直接报 provenance 错误。输出还必须有实际 Runtime 载入标记、新的 Profile audit、native free 和符合记录的预期状态，并检查该 attempt 的 Host fallback 审计为零。

初次结果和退出码不修改；复验成功也不能把第一次的 deadline 改写为成功。复验本地场景时必须单独提供仍在运行的受控 listener；不能把关闭的端口当作原来的成功夹具。原本预期全部失败的端口场景可按记录的失败状态复验。公共查询的新审计仅证明产品观测范围，不是独立流量捕获。

## 同进程与父子原生 harness

```powershell
cmake -S tools/envbox-dns-unified-fixture -B target/dns-unified-harness64 -G "Visual Studio 17 2022" -A x64
cmake --build target/dns-unified-harness64 --config Release
cmake -S tools/envbox-dns-unified-fixture -B target/dns-unified-harness32 -G "Visual Studio 17 2022" -A Win32
cmake --build target/dns-unified-harness32 --config Release
python tools/envbox-dns-unified-fixture/run_resources.py `
  --cli target/doh-integrated-clients/debug/envbox.exe `
  --harness64 target/dns-unified-harness64/Release/envbox-dns-unified-harness.exe `
  --harness32 target/dns-unified-harness32/Release/envbox-dns-unified-harness.exe `
  --runtime-dir <frozen-reviewed-runtime-pair>
```

每种传输的每个架构在一个真正注入进程内先执行 8 次 warmup，再串行 8 次，然后 4 线程各 8 次重复两批，共 80 次父进程 TXT 查询；最后通过实际 `CreateProcessW` 创建 child，由 child 验证 Runtime 载入、Profile/Instance 与 parent 相同，并执行 8 次 TXT 查询。受控 UDP/TCP 检查 TXT 的固定 `profile-marker`，公共服务检查真实 TXT 记录。

独立取消 phase 每架构执行 4 次 warmup 和 32 次实际异步取消，复制 cancel handle 后重复取消，检查单次回调与状态 1223。所有 DNS 记录均用 native free 释放；请求、结果与回调 context 保持存活至完成。如果回调超过安全等待上限，直接结束当前 harness 进程并报错，不返回到已经失效的栈存储。

采样记录 warm/repeat/两批 concurrency 或取消前后 `GetProcessHandleCount` 与 private bytes。短批次门禁是第二并发批次或取消结束相较基线最多增加 4 个 handles、2 MiB private bytes，不能解释为无限长跑或生产负载资格。runner 对 parent/child 的实际 Runtime 模块文件重新计算 hash，并与传入冻结 pair 比较；保留全输出、PID、Profile/Instance、受控 request wire。

这补充父子 DNS 与短批次资源证据，仍不包含实际 GUI 打开→退出、特殊代启动方式或独立 Host DNS 流量观测。

## 2026-10-06 实际结果

初次四传输矩阵使用冻结 `doh-integrated-runtime-final-guard` pair，新鲜 WMI controller PID 13252 检测 Runtime modules 0：

- `target/dns-unified-9c181ee113034ea79b9481aec5f77a85/result.json`：464 项中 460 通过、4 个公共 DoT deadline，wrapper exit 1。受控 UDP/TCP 220 项、顺序/全失败/取消/deadline 24 项、公共 DoH 110 项全部通过；公共 DoT 106/110。失败是 x64 TXT/CNAME async 和 x86 SRV A/NS UTF8，审计均为 `dot-deadline`，没有证书错误或 Host fallback。
- 独立原始 12 次复验保留在 `target/dns-unified-repeat-e75d014eda7f42f2a2d41d76e6112900/result.json`，12/12；其结果早于新增 provenance 门禁，不能冒充新门禁证据。
- 修复复验工具后，新鲜 WMI PID 7036 再执行四个失败用例各一次，`target/dns-unified-repeat-9e9347aad6af4c9380fd6ea13e4025e9/result.json` 为 4/4、exit 0。每次都有实际 before/after actor hash、Runtime loaded、新审计及 Host fallback 0。初次失败记录未修改。

最终 native resource/child 全矩阵使用冻结 `with-token-latched-runtime`，新鲜 WMI controller PID 4072 检测 Runtime modules 0：

- `target/dns-unified-resources-dba8d499f6954c5a88171f246159b507/result.json`：全部 10 个 phase 的资源门禁稳定；8/8 父子同 Profile/Instance、实际 DLL hash 匹配且 child DNS 成功；72/72 异步取消为单次回调和 1223。普通 TXT 共 704 次中 x86 DoT 有一次 deadline，其余成功。因此整体 exit 1，保留该公网超时，不显示全部功能通过。
- x86 DoT 超时对应 PID 30512，审计 `be87f7cd-0206-442b-853a-87daf8fa9082.jsonl` 中为 `dot-deadline`。资源和父子结果分别记录，不把公网 deadline 当作资源泄漏。没有进一步反复执行同一公共负载。
- 全部 phase 的 Host fallback 审计为 0。受控 UDP/TCP 每个重复 phase 均收到了 88 个请求；取消可在发送之前完成，因此不要求取消请求都到达服务端。

最终 Runtime SHA-256：

- x64：`74DF21C7867A95B7DBF8EE6D625614BC00CCA1147B6A43504A0447EDB3F1CC20`
- x86：`064490B6A831B9F2F3C24D0B052C0674D6DA96713896A13418A1A72739E34C22`

复验工具的 Profile mapping 门禁还有实际 CLI 控制证据：旧 `dead64` 映射、新记录的 Profile ID、旧 `ordered64` 映射均已运行。ordered 控制由新鲜 WMI PID 22772 拥有原端口 UDP listener，`target/dns-unified-repeat-07bd536960d84b5d8d9bade98017a21a/result.json` 为 1/1，独立 `target/dns-unified-repeat-ordered-wire.json` 为一个请求且 worker 已停止。seed 明确标为工具映射控制，不能解释为新的产品失败。

provenance 负向控制故意只修改 seed 中预期的 CLI hash，不修改任何 EXE。新鲜 WMI PID 19568 的 `target/dns-unified-repeat-provenance-negative-exit.json` 为预期拒绝 exit 1，`target/dns-unified-repeat-b9a4d371a30c4a0f8799a98f64f621c4/result.json` 保存具体 expected/actual hash 和零已执行 rows；在 query 之前拒绝，日志有清晰 provenance 错误。

上述是当前 IPv4 支持范围的行为证据，IPv6 已延期。GUI 退出、独立流量捕获、更多 resolver 入口和生产长跑仍不能由这些结果推断完成。
