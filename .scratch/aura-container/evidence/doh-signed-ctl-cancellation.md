# DoH native cache 取消与 signed CTL 受控验签

Date: 2026-10-06
Baseline: `b193fe933b758143fbbe6543c5d701f9576be381`
Branch: `dev`
Status: partial / ticket-18-No-Go / ticket-19-off

## 本切片实现

native cache collector 已恢复原调用者的完整 Budget。query 在调用线程创建非 Send RAII scope，直到 current-thread runtime 与全部连接任务清理后才退役。Send + Sync verifier 只保存 ThreadId 与该线程单调递增且不回绕复用的 scope ID，不保存 callback/context。scope 不存在、已退役或线程不符时直接拒绝；TLS registry 借用在外部 callback 前释放，支持嵌套 scope。

collector 在同步 CAPI 步骤间检查 callback/deadline；首次 Cancelled/Deadline 锁存到 scope。transport 完成清理后优先返回该类型，避免一次性取消被 rustls General 错误映射吞掉。单个同步 CAPI 内部仍不能抢占，不声称固定的取消延迟上限。无后台 callback worker、unsafe Send/Sync、Host fallback 或 cache/store 写入。

新增 `signed_ctl` 研究模块，仅 `cfg(test/fixture-trust)` 编译。显式 DER signer pins 加入 memory-only store，每个 signer index 使用 `CMSG_TRUSTED_SIGNER_FLAG | CMSG_USE_SIGNER_INDEX_FLAG` 实际验签；不得只比较 signer 身份或信任消息附带证书。全部 signer 验签及 RSA/SHA-2、Root List Signer EKU、有效期检查通过后，才解释 CTL 的 usage、list identifier、时间、sequence 与 subject identifiers。sequence 回退、相等 sequence 不同 encoded SHA-256、未知属性/算法/结构和过期材料严格拒绝。输出只含拥有的数据。

17 份可重建 fixture 共约 21 KB，只持久化公开 DER/STL，无私钥、在线下载或宿主 trust 修改。真实 AuthRoot policy attributes、pins 来源、publisher rotation、signer chain/revocation、root materialization、持久化 antirollback 和完整生产授权均未实现。本模块不改变默认 ROOT loader，不把受控成功升级为公共 AuthRoot 成功。

## 验证

- 审查修正前 Rust 1.99 x64/i686 DoH crate 各 **24 passed / 0 failed / 1 ignored**，日志 `target/doh-signed-scope-tests64.log`、`target/doh-signed-scope-tests32.log`。随后补充 CAPI actual-size/embedded pointer/OID 边界检查与纯 helper 测试，最终 i686 crate **25 passed / 0 failed / 1 ignored**，日志 `target/doh-signed-scope-reviewed-tests32.log`。signed CTL 定向双架构各 5 tests 通过，包含真实验签正反例；完整工作区最终结果另列。
- 双架构 fixture **92 个 IPv4 场景通过**，完整保留原 84 场景；fresh WMI controller PID 4932，Runtime modules 0。`target/doh-cache-cancel-matrix-result.json`、`target/doh-cache-cancel-matrix.log`、`target/doh-cache-cancel-matrix-host.json`。
- IPv6 preflight 10013；用户关闭 IPv6，**10 个 IPv6 场景未执行**，不计为通过，没有修改宿主配置。

真实 native cache-only collector 的新增场景，每架构均执行：

| 场景 | 返回 | cache checks | cancel pulses | 查询后 callback |
| --- | --- | ---: | ---: | ---: |
| cache miss | RevocationUnknown 7 | 12 | 0 | 0 |
| collector entry | Cancelled 2 | 1 | 1 | 0 |
| 第一次 CDP CAPI 返回后 | Cancelled 2 | 5 | 1 | 0 |
| cache-only retrieval 返回后 | Cancelled 2 | 10 | 1 | 0 |

全部场景 HTTP/canary 0、18 项禁用 Host API 0、UDP 0、拒绝 endpoint 0。取消只触发一次且随后恢复零，证明返回类型由锁存维持；测试按阶段计数触发，不依赖时间窗口。只证明该进程 instrumentation 与 loopback 范围，不能代替完整系统网络捕获。

fixture 可执行 SHA-256：

- x64 `A3FA3B984469EE1E7979EA88231A78CEC9F620FCAF1A6BBADB5226C52343D6E5`
- x86 `2AFB58361D142101C79FBBBE1AC630B6B6B6C5761410F50740C09C5A86C2AF39`

## 默认 native 产品路径复测

默认 staticlib → MSVC DLL → C ABI RunId `dc449b05742c41e4b86f2d986df4a342`，fresh WMI PID 28664 / Runtime 0；x64 Host 23240、x86 Host 28856，均 Runtime 0。双架构 build/link/symbol exclusion、Host/worker exit 0；wrapper exit 1 来自完成后的 positive gate=false。Cloudflare/Google IPv4 均 certificate 6、响应长度 0，IPv6 network 4。bridge 传 nullptr callback/context，因此本次 native C ABI 只验证无回调默认路径，不替代上述缓存取消矩阵。

默认诊断 RunId `9eed021878e3482abfc47860d00d1728` / PID 24084 / Runtime 0，双架构 `UnknownIssuer`；AuthRoot 研究诊断 `8a3dfb1e3d8741b5ae23edfa6740b7e1` / PID 1472 / Runtime 0，双架构 `UnknownRevocationStatus`。Google cache 候选 2、Cloudflare 0，均不提升为已认证吊销材料；IPv6 10051。诊断 wrapper exit 0 表示完成，不表示信任通过。

| 默认 native 产物 | SHA-256 |
| --- | --- |
| x64 MSVC DLL | `55E5929B74FFAEBFFC86B0382C771755F29EA49CB45D7287814EFB9433E8B116` |
| x86 MSVC DLL | `58D22F3220CC5D8FF13D5E2B590443D5AEC8AF5833E8E3ACC1D87941E09B00F4` |
| x64 staticlib | `5A87E090F3E76C7209A5DE9F097DE7307065625D9F7612C9A15E1EBF00C22EC1` |
| x86 staticlib | `B6C946617BA48975C09874AE07FD73F26DA4A1CBBC1308027A5C05AA5EE77961` |

唯一 JSON/架构日志分别以以上 RunId 保留于 `target/doh-native-acceptance-*`、`target/doh-native-diagnostic-*`、`target/doh-native-diagnostic-authroot-*`。正式 Runtime 冻结 DLL pair 未改，DoH 未启用；原型 staticlib/MSVC DLL 与该 Runtime pair 是不同产物。

## 剩余门禁

步骤间取消缺口已补齐；公共离线信任、真实 AuthRoot 授权、完整辅助流量观测及目标 OS 矩阵仍未满足。票 18 保持 No-Go，19 保持 off。不在线补材料、不关闭 revocation、不退回系统 DNS。P4–P8 驱动加载、Verifier、WFP/overlay 与内核故障恢复仍需隔离 VM；本切片不对其作运行声明。

独立 Standards/Spec 结果见 [review](doh-signed-ctl-cancellation-review.md)，后续研究边界见 [AuthRoot 研究](authroot-offline-research.md)。未发布或加载驱动，未修改宿主网络/trust。

## 审查修正与验证来源

Standards 发现并修正 research CTL 两阶段 CAPI output buffer 的范围缺口：header 最小值、actual size 与 allocated 上界均在结构解引用前检查；embedded pointer/OID/algorithm blob 使用实际返回范围。纯 helper 测试覆盖 short actual、超分配、实际范围之外指针、地址溢出、未终止 OID 和有效数据。该修正只在 test/fixture 模块，不改变已复测的默认 native staticlib 代码或缓存取消 collector。92 场景验证也不调用 signed CTL 组件。

修正前完整回归已通过 **413 passed / 0 failed / 34 ignored**，fresh WMI PID 6700 和确认运行 PID 20848 均 Runtime 0、build/test exit 0。对应 `target/workspace-signed-scope-final-*` 与 `target/workspace-signed-scope-confirmed-*`。最终修正后的回归单独记录，历史日志不覆盖。

最终修正后 Rust 1.99 `cargo build --locked --workspace` 与 `cargo test --locked --workspace --no-fail-fast` 均 exit 0，实际 **414 passed / 0 failed / 34 ignored**。fresh WMI controller PID **5584** / Runtime modules **0**，2026-10-06 09:31:16Z 至 09:32:46Z；来源 `target/workspace-signed-scope-reviewed-result.json`、`-build.log`、`-test.log`、`-controller.log`。其中 x64 DoH crate **25/0/1**，最终 i686 crate亦 **25/0/1**。

工作区原生测试使用之前冻结的 Runtime pair（本切片未修改 C++ Runtime）：x64 SHA-256 `CA8283ADAE000DBEAAE65902A10F2E0E05B94C38C14F7B3652EDD3066C43D965`，x86 `5D2FD1038748F0D579F5B2EB59EBAEECDFF6C7846D12FD93876875696B6CEBE7`。不能用该 pair 宣称已启用新 DoH 原型。限定 Rust 文件 `rustfmt --check`、两个 Python 脚本 AST 和 scoped `git diff --check` 通过；独立复审确认 P2 resolved。
