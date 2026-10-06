# DoH cache cancellation / signed CTL 双轴 review

Date: 2026-10-06
Baseline: `b193fe933b758143fbbe6543c5d701f9576be381`
Branch: `dev`
Scope: 本切片 working-tree 与新增文件；caller-thread cancellation、研究用 signed CTL、fixture/diagnostic 和证据更新。
Status: reviewed / P2-resolved / partial / ticket-18-No-Go

按 implement / code-review Skill，由两名独立只读 Agent 分别审查 Standards 和 Spec；主 Agent 复核关键生命周期、unsafe 边界、diff 和实际运行日志。不把全部 Container 总规格的未完成项隐藏在切片结果中。

## Standards

发现一个 P2：signed CTL 的两阶段 `CertGetEnhancedKeyUsage` / `CryptMsgGetParam(CMSG_SIGNER_INFO_PARAM)` 在第二次调用后缺少 `header <= actual size <= allocated` 检查，随后读取的 caller-owned buffer embedded pointers/OID/参数也缺少实际返回范围检查；`CStr::from_ptr` 是无界扫描。成功 CAPI 的结构合约不能替代 Rust wrapper 自身的有界 unsafe 读取。

其余审查通过：query scope 为非 Send、token 非复用、foreign/retired fail closed、外部 callback 前释放 registry borrow、嵌套重入、stop latch、每步原 callback 传播、fixture 无私钥/Host writes 和 product feature 隔离均符合要求。没有新增判断型 smell finding。

## Spec

本轮源码没有新的 actionable finding。scope 不保存 callback/context 到 verifier，query 清理后精确退役；collector 停止类型锁存并在 transport 优先传播，未被 TLS 通用错误吞掉。真实 cache-only phase pulse 双架构得到 Cancelled 2，查询后 callback 恢复零，HTTP/canary 为零。

signed CTL 使用显式 pins 的 memory store，每个 signer index 真实验签，全部通过后才读取授权 metadata；未知属性、算法、EKU、时间、identity 和 rollback 失败边界保留。研究模块仅 test/fixture 可见，不暗中增加生产 AuthRoot 信任。

完整规格仍 partial：实际 AuthRoot signer chain、rotation/revocation、跨版本 policy attributes、root materialization 和生产接入尚未完成；单个同步 CAPI 不可抢占。IPv6 10 项未执行；18 No-Go、19 off；P4–P8 内核运行门禁仍需隔离 VM。

## 验证与交付边界

源测试、fixture runtime、默认 native C ABI 与公共信任结果分别记录于 [实现证据](doh-signed-ctl-cancellation.md)。默认 C ABI nullptr callback 不被当作 callback 生命周期证明。未发布、未加载驱动、未修改宿主 IPv6/trust/network。

独立审查统计：Standards 1 个 P2；Spec 0 个新增 actionable finding，完整要求仍 partial。最终修正与复验结果补在本节之后，不改变历史发现记录。

## 最终修正复审

Standards reviewer 已独立复核修正：两个 CAPI 的第二次返回均在结构解引用前使用 `Region::returned` 检查 actual 长度；EKU 数组整体与各 OID、signer algorithm OID/parameters 均限制到 actual returned region；API-owned decoded context 的 OID 改为有界读取，移除无界 CStr；范围加法 checked，helper tests 覆盖短实际结果、超分配、区域外指针和未终止字符串。原 P2 标记 resolved，未发现新实际问题。

主 Agent 复核相同关键 diff、双架构真实签名正反例和 i686 全 crate 结果。最终 Standards 未解决 actionable findings **0**；Spec 新增 actionable findings **0**，完整 Container/DoH 要求仍 partial。验证数字和 fresh-host provenance 以实现证据最终记录为准。

最终修正后工作区 build/test exit 0，**414 passed / 0 failed / 34 ignored**，fresh WMI PID 5584 / Runtime 0；x64/i686 DoH crate 各 **25/0/1**。92 个 IPv4 fixture 场景通过，10 个 IPv6 未执行；默认 native C ABI 和独立信任诊断均保持预期 No-Go。限定格式/脚本语法/diff 检查通过。修正仅研究模块，未改变 native 默认库或矩阵所测 cancellation source。
