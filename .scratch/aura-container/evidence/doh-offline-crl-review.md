# DoH 只读 CRL 切片的双轴 review

Date: 2026-10-06
Baseline: `af1f6f681cd523c79709c69677870db6334865d8`
Branch: `dev`
Scope: 本轮 staged/working-tree 中的 CRL adapter、严格 cache 重试、fixture/diagnostic、AuthRoot 研究和证据。
Status: reviewed / partial-acceptance / ticket-18-No-Go

按项目 code-review Skill，由两名独立只读 Agent 分别执行 Standards 与 Spec，基线为本轮开始的 HEAD。原有容器总规格、DNS transports 规格、票 18/19 和明确的 cache-only 增量设计共同约束本切片；不把尚未实现的全部 P0–P8 升级为通过。

## Standards

最终代码审查没有发现新的高风险问题：固定 cache-only/no-write flags、context RAII、actual buffer 区域校验、空 CRL 不得接受、retry allowlist 和 fixture/product feature 隔离均符合当前约束。

审查的 actionable finding 已逐项处理：

- CDP 二阶段 `size == estimate` 会误拒合法短结果：修正为 header 最小尺寸至 allocated 范围内，embedded pointers/string 使用实际返回长度校验。修正后 i686 实际 crate tests、双架构 fixture、workspace 和默认 C ABI 都重新验证。
- native cache 丢弃 caller cancellation callback：**没有宣称已实现及时取消**。这是明确保留的正式启用门禁；只保存 deadline，transport 在 TLS 返回后、HTTP 前和销毁后检查取消。票 18、证据、adapter header 和 crate README 均明确该边界。未用 unsafe Send/Sync 延长借用 callback。
- README 的 blanket cancellation 保证、证据中含混的“超时/取消覆盖”收窄为真实范围：adapter 的带 callback Budget 入口拒绝与 native verifier deadline-only 阶段分开记录，fixture cancellation 不能作为后者的通过证据。
- review 文档断链由本文件补齐。

判断型低优先级 smell：`trust.rs`/`offline_crl.rs` 的 `Cert/Crl` RAII wrapper 重复。两个模块所有权清晰且只包含对应释放操作，本轮保留，不增加尚无实际需求的通用资源抽象。

## Spec

审查确认没有 Host fallback、wire retrieval 或 cache/store 写入；缓存 DER 不授权 root，仍需标准 verifier 完整检查。空 CRL、未知 issuer、错名、revoked 等失败边界保留。CDP 尺寸误拒 finding 按上述修正。

原审查指出缓存重试分支缺乏正反向覆盖：已增加 fixture-only DER 候选输入，双架构新增 26 个场景，覆盖有效候选、stale 刷新、miss、错误 issuer/签名/过期、revoked EE/CA、完整链和缺 CA revocation、不能覆盖已有 revoked、错名及不可信链。实际 84 个 IPv4 场景通过；不读写真实 cache，不把该正向升级为 product/native cache 公共 TLS 正向。

完整验收仍是 partial：默认公共 TLS `UnknownIssuer`，AuthRoot 研究变体 `UnknownRevocationStatus`；已有真实 cache hit 不能证明全链撤销。native cache 及时取消、AuthRoot CTL 验签/策略/signer、完整辅助网络观测与 OS 矩阵仍未完成。IPv6 10 项因用户禁用而未执行；驱动加载、Verifier、WFP/overlay 与 P4–P8 故障恢复仍需隔离 VM。票 18 No-Go/19 off 保留。

## 必要验证

- Rust 1.99 whole workspace：最终 fresh WMI Host PID 17372 / Runtime modules 0，build/test exit 0，403 passed / 0 failed / 34 ignored。
- i686 DoH crate 实际 tests：14 passed / 0 failed / 1 ignored。
- 双架构 DoH fixture：最终 fresh WMI Host PID 30664 / Runtime modules 0，84 个已执行 IPv4 场景通过，10 个 IPv6 场景未执行。
- 默认 product staticlib → MSVC DLL → C ABI：最终 RunId `61b07c27a4a84e339f2bba5935e62201` / Host PID 9548 / Runtime modules 0，双架构构建/链接/调用完成；公共 IPv4 仍 certificate failure，无响应，预期 No-Go。
- 原生诊断最终默认/研究 RunId 分别 `56317ca35d87400fb0c9fa62a984ce1f` / `2c77fed9738843b6b07871d36cab412d`，结果仍 No-Go；Google 命中 2 个未认证 CRL 候选，Cloudflare 0。

最终来源、hash、日志、复现方式和启用门禁见 [实现证据](doh-offline-crl-cache.md)。未发布、未加载驱动、未修改宿主 IPv6/网络/trust。

总计：Standards 0 个未处理的本轮 actionable finding；Spec 0 个未处理的本轮 actionable finding。最严重的剩余支持门禁分别为 native cache 及时取消及公共原生信任正向，均未实现/未通过，18/19 不放行。
