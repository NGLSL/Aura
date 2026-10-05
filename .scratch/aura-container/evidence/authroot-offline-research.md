# Windows AuthRoot CTL 安全离线使用研究

Date: 2026-10-06
Status: research-complete / implementation-gated
Scope: EnvBox/Aura 的离线 TLS 信任材料（DoH/DoT），只读研究，不改生产代码或测试。

## 结论

AuthRoot **可以被安全地作为离线输入验证和按需 materialize**，但它不能被解释成“把 Windows logical `Root` 或全部 AuthRoot 证书导入 Aura 的 RootCertStore”。最小可靠方案是一个有版本的、可审计的离线 trust snapshot：

1. 在受控的获取步骤中取得 `authrootstl.cab`/`authroot.stl`、对应根证书和 Disallowed CTL；获取步骤可以使用 Microsoft 文档描述的同步工具，但不属于 Runtime/Profile 的网络路径。
2. 在 Aura 内只处理调用者显式提供的字节。先用 `CertCreateCTLContext` 解码，再用 `CryptMsgGetAndVerifySigner` 对 CTL 的签名做 detached verification；签名验证使用只包含显式受信 signer 的 memory store，并要求 signer 身份和用途匹配。
3. 独立检查 CTL 的 `ThisUpdate`、`NextUpdate`、`SequenceNumber`、`ListIdentifier`、签名算法、所有 critical/未知属性，以及对应根证书的 X.509 有效期、CA/EKU、Disallowed、NotBefore/NotAfter 和名称/链策略。
4. 只将请求所需、与 CTL entry 精确匹配、所有策略都可解释的根放入本次 TLS verifier。任何未识别的 AuthRoot policy、缺失的 signer/根证书、过期或未来 CTL、未知撤销状态都 fail closed。
5. 通过 CryptoAPI 时创建 restricted/custom chain engine，并显式禁止 AIA、AuthRoot 自动更新和 URL retrieval；通过 Rustls 时把经过同一 gate 的证书 DER 作为输入。两条路径都禁止回退到系统 DNS、在线 CRL/OCSP 或“忽略 revocation”。

这条路径能解决“缺少 Cloudflare SSL.com ECC 或 Google GlobalSign root”这一类 anchor 缺口，但**当前机器仍不能证明完整方案已通过**：把 AuthRoot 候选直接加入研究 snapshot 后，公共 DoH 的失败从 `UnknownIssuer` 推进到 `UnknownRevocationStatus`。当前 CRL/OCSP 快照不足以证明严格离线撤销，因此不能用全量 AuthRoot 或放宽 revocation 作为修复。

公开 Microsoft 文档提供了 CTL 签名校验、memory store、CTL freshness 字段和 chain-engine 的离线开关；但没有给出一个跨所有 Windows 版本、可由第三方手工重实现的完整 AuthRoot root-program policy wire schema，也没有保证 signer 证书永不轮换。因此安全实现必须把未能由受支持 API 和版本化 fixture 证明的字段当作“不支持”，而不是猜测。

## 证据与资料边界

下表只引用 Microsoft 官方文档、Microsoft 官方 Trusted Root Program 仓库或本机 Windows SDK。链接旁的结论只覆盖该链接明确承诺的行为。

| 主题 | 一手来源 | 本记录采用的事实 |
| --- | --- | --- |
| AuthRoot/Disallowed 的 Windows 管理和自动更新 | [Certificate trust and trusted roots](https://learn.microsoft.com/en-us/windows-server/identity/ad-cs/certificate-trust)、[Configure trusted roots and disallowed certificates](https://learn.microsoft.com/en-us/windows-server/identity/ad-cs/configure-trusted-roots-disallowed-certificates) | Windows 维护 AuthRoot 和 Disallowed CTL；CTL、CAB、根证书和本地 registry/cache 是不同层次的材料；同步需要网络，离线机器可以把材料通过文件/可移动介质带入。 |
| CTL 签名验证 | [Verifying a CTL](https://learn.microsoft.com/en-us/windows/win32/seccrypto/verifying-a-ctl)、[CryptMsgGetAndVerifySigner](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-cryptmsggetandverifysigner) | 每次使用 CTL 前必须验证签名；`CMSG_TRUSTED_SIGNER_FLAG` 将 signer store 限定为调用者提供的 store；`CMSG_SIGNER_ONLY_FLAG` 会跳过签名校验，不能使用。 |
| CTL 数据结构 | [CTL_CONTEXT](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/ns-wincrypt-ctl_context)、[CTL_INFO](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/ns-wincrypt-ctl_info)、[CTL_ENTRY](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/ns-wincrypt-ctl_entry) | `CTL_CONTEXT` 暴露已解码 CTL 和 `HCRYPTMSG`；`CTL_INFO` 包含 usage、list identifier、sequence、`ThisUpdate`/`NextUpdate`；entry 是 identifier 加 attributes。 |
| CTL usage dispatcher | [CertVerifyCTLUsage](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certverifyctlusage)、[CTL_VERIFY_USAGE_PARA](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/ns-wincrypt-ctl_verify_usage_para) | 默认 signer 搜索范围很宽；`CERT_VERIFY_TRUSTED_SIGNERS_FLAG` 才限制到 usage 参数里的 signer stores；未抑制更新时，过期 CTL 可能触发替换/更新路径。因此它不应未经约束直接作为 Runtime 的离线 gate。 |
| memory store 与非持久化 | [CertOpenStore](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certopenstore)、[CertCreateCTLContext](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certcreatectlcontext)、[CertAddEncodedCTLToStore](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certaddencodedctltostore) | `CERT_STORE_PROV_MEMORY` 不会写 system registry；`CertCreateCTLContext` 只从编码字节创建 context；需要持久化的 provider 应避免在 Runtime 中使用。 |
| chain 的离线开关 | [CertGetCertificateChain](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certgetcertificatechain)、[CERT_CHAIN_ENGINE_CONFIG](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/ns-wincrypt-cert_chain_engine_config) | `CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL` 禁止 URL retrieval；`CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY` 只用已有撤销 cache；`CERT_CHAIN_DISABLE_AIA` 禁止 AIA；`CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE` 禁止 AuthRoot 自动更新；这些 flags 不会把缺失 cache 变成“已撤销检查通过”。 |
| AIA 是网络入口 | [Authority Information Access retrieval](https://learn.microsoft.com/en-us/windows-server/security/authority-information-access-retrieval) | Windows 可能从 AIA 下载缺失的 intermediate；Aura 必须用每个 custom chain engine 的 flag，不能改全局 registry。 |
| root program 的当前规则与撤销语义 | [Microsoft Trusted Root Program requirements](https://github.com/TrustedRootProgram/Program-Requirements)、[Root certificate deprecation](https://learn.microsoft.com/en-us/security/trusted-root/deprecation) | Remove、Disable、EKU Removal、NotBefore 的效果不同；不能把所有“当前在 CTL 中出现”的 entry 当成无限期、全用途 trust anchor。 |

## 当前机器的只读观察

这些是 2026-10-06 同日早期已保存的本机只读诊断观察；本研究复用该证据，未重新同步或下载。它们不是跨 Windows 版本的 API 契约，也不代表之后的缓存仍保持相同内容。

```text
certutil -verifyCTL AuthRoot
LastSyncTime  = 2026/10/6 2:23
SequenceNumber = 1401dd3462c779aec7
ThisUpdate    = 2026/8/25 15:24
SubjectAlgorithm = 1.3.14.3.2.26 (SHA-1)
CTLEntries    = 562
```

命令未使用 `-f`，所以这次观察没有把“为了完成命令而强制下载”混入证据。Microsoft 的 [`certutil` 命令文档](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/certutil)将 `-verifyCTL AuthRoot`、`AuthRootWU`、`-syncWithWU` 分开：前者验证已有 AuthRoot 内容，后者才是显式同步/强制获取路径。这里的 `LastSyncTime` 也不能代替 CTL 自身的 `ThisUpdate`/`NextUpdate` 校验。

本机 `certutil -verifyCTL AuthRoot` 返回成功，只能证明本机 crypt32 能识别和验证这份 AuthRoot 材料。它不能证明：

- Aura 的 Rustls trust snapshot 已经采用了这份 CTL；
- 所有 562 个 entry 都有对应的根证书 DER；
- 当前目标 TLS 链的 intermediate 有新鲜 CRL/OCSP；
- 把全部 AuthRoot 作为 trust anchor 能满足严格撤销要求。

现有公共 DoH 诊断还记录到：默认物理 `ROOT` 中缺少 Cloudflare 当前 SSL.com ECC root 和 Google 当前链所用 GlobalSign root；把 AuthRoot 候选根只读加入研究 variant 后，错误由 `InvalidCertificate(UnknownIssuer)` 变为 `InvalidCertificate(UnknownRevocationStatus)`。这支持“先解决受控 anchor materialization，再解决离线撤销材料”的分层判断，但不支持全量导入。

## AuthRoot 材料和信任来源的分层

### 1. CTL、CAB、根证书和系统 store 不是同一件事

Windows 文档描述的 AuthRoot 更新包通常包括：

- `authrootstl.cab`：传输/分发包；
- `authroot.stl`：包含 trusted roots 的 CTL；
- `authrootseq.txt`：更新序列信息；
- 按 entry 提供的 `.crt` 根证书；
- `HKLM\SOFTWARE\Microsoft\SystemCertificates\AuthRoot\AutoUpdate\EncodedCtl` 等 cache/registry material。

本机 SDK `wincrypt.h`（Windows 10 SDK 10.0.26100.0）定义了上述 AuthRoot 文件和值名称：`CERT_AUTH_ROOT_CTL_FILENAME`、`CERT_AUTH_ROOT_CAB_FILENAME`、`CERT_AUTH_ROOT_SEQ_FILENAME`、`CERT_AUTH_ROOT_AUTO_UPDATE_ENCODED_CTL_VALUE_NAME`。这些名称是 SDK 的 Windows 集成契约；它们不能推出 Aura 可以绕过 CTL usage/policy 直接把 `AuthRoot\Certificates` 变成自己的根库。

### 2. 签名有效和 signer 受信是两个检查

Microsoft 的 [Verifying a CTL](https://learn.microsoft.com/en-us/windows/win32/seccrypto/verifying-a-ctl) 示例要求先取得 `CTL_CONTEXT`，再通过 `CryptMsgGetAndVerifySigner` 验证签名。`CTL_CONTEXT->hCryptMsg` 是 cryptographically signed inner message，适合做 detached verification。

`CryptMsgGetAndVerifySigner` 的关键边界是：

- `CMSG_TRUSTED_SIGNER_FLAG` 表示只搜索调用者传入的 signer store，并把这些 store 中的证书视为受信 signer；
- 没有该 flag 时，crypt32 可能搜索更广的 store，不能把“系统中能找到 signer”当成 Aura 的 pinned trust decision；
- `CMSG_SIGNER_ONLY_FLAG` 仅取 signer，不验证签名，严格模式禁止；
- 返回的 signer context 仍需比对已允许的 DER/SPKI/hash、用途、有效期和算法，而不是只看 API 返回成功。

Windows SDK 在本机 `wincrypt.h` 3464–3465 行把 `1.3.6.1.4.1.311.10.3.9` 定义为 `szOID_ROOT_LIST_SIGNER`，注释为“Signer of a CTL containing trusted roots”。SDK 还定义 `1.3.6.1.4.1.311.10.3.30` 为 `szOID_DISALLOWED_LIST`。这可以作为检查 signer EKU/CTL usage 的 Windows 依据，但 SDK 头文件**没有提供一个永远不变的 signer 证书 thumbprint 清单**。

因此，最小可实现的 signer gate 是：

1. 将允许的 Microsoft Root List Signer 证书以版本化 DER/SPKI hash 作为输入，来源、轮换和撤销由发布材料单独记录；
2. 对 `CTL_CONTEXT->hCryptMsg` 指定 signer index，逐个验证预期 signer；不接受“任意一个能验签的 signer”作为结果；
3. 传入只含这些 signer 的 memory store 和 `CMSG_TRUSTED_SIGNER_FLAG`；
4. 验证 signer 的链/有效期/签名算法及 `szOID_ROOT_LIST_SIGNER` 用途；
5. signer 缺失、轮换未被发布材料承认、多个 signer 中出现未预期 signer 或签名算法不在允许集合时，拒绝整份 CTL。

目前不能把“系统 Root store 里找到的 Microsoft 证书”自动当成第 1 步的 signer pin。Microsoft 的 [Event ID 4107/11](https://learn.microsoft.com/en-us/troubleshoot/windows-server/certificates-and-public-key-infrastructure-pki/event-id-4107-or-event-id-11-is-logged) 文档还说明，Microsoft Certificate Trust List Publisher 证书过期会导致 AuthRoot 更新失败；这足以说明 signer 生命周期不能假设为永久固定。

### 3. freshness 必须来自 CTL 自身

`CTL_INFO` 明确提供：

- `ThisUpdate`：本份列表的发布时间；
- `NextUpdate`：下一次更新时间界限；
- `ListIdentifier`：列表身份；
- `SequenceNumber`：同一列表的序列号。

strict offline gate 应当：

- 拒绝无法解码或没有可用 `NextUpdate` 的 CTL；
- 在固定的、受测试的 clock-skew 容忍范围外，要求 `ThisUpdate <= now < NextUpdate`；
- 对同一 `ListIdentifier` 拒绝低于已接受 snapshot 的 `SequenceNumber`，避免离线回滚；
- 以 CTL 的字段为准，不把 registry `LastSyncTime` 或 CAB 文件时间当作 freshness；
- 不使用 `CERT_VERIFY_NO_TIME_CHECK_FLAG`；
- 记录 CTL bytes、签名者 fingerprint、list id、sequence、时间和输入 hash，便于审计。

这里的“序列号防回滚”是 Aura 对 snapshot 的安全约束；它不声称 crypt32 的所有 provider 都以同样方式暴露持久化状态。每个 Windows 版本/SDK 应通过 fixture 证明 `SequenceNumber` 的编码和比较规则。

## AuthRoot policy、usage 和 entry attributes

### 能够由官方接口直接读取的字段

`CTL_ENTRY` 的 `SubjectIdentifier` 和 attributes 是正式结构字段。当前 AuthRoot CTL 的本机观察使用 SHA-1 `SubjectAlgorithm`，但实现必须读取 `CTL_INFO.SubjectAlgorithm` 并据此解释 identifier，不能把 SHA-1 永久写死。

Microsoft SDK 还提供把证书 context properties 编码为 CTL entry attributes、以及把 entry attributes 还原为证书 context properties 的 API。`wincrypt.h` 对 `CertCreateCTLEntryFromCertificateContextProperties` 的注释说明：property attribute OID 是 `1.3.6.1.4.1.311.10.11.` 加十进制 property id；对应的 `CertSetCertificateContextPropertiesFromCTLEntry` 只复制带该 OID 的属性。这个本机 SDK 注释是理解 wire 形态的关键，但仍需针对每个目标 Windows SDK/fixture 做兼容测试。

本机 SDK 中与 Root Program/禁用相关的 property id 如下：

| SDK symbol | property id / 由 `szOID_CERT_PROP_ID` 形成的 OID | 语义边界 |
| --- | --- | --- |
| `CERT_ROOT_PROGRAM_CERT_POLICIES_PROP_ID` | `83` / `1.3.6.1.4.1.311.10.11.83` | Root Program certificate policies；必须按其编码解码，不应当视为普通 EKU。 |
| `CERT_ROOT_PROGRAM_NAME_CONSTRAINTS_PROP_ID` | `84` / `.84` | Root Program name constraints；未知或无法严格应用时拒绝该 root。 |
| `CERT_DISALLOWED_FILETIME_PROP_ID` | `104` / `.104` | Disallowed 时间属性。 |
| `CERT_ROOT_PROGRAM_CHAIN_POLICIES_PROP_ID` | `105` / `.105` | SDK 注释规定编码为 `X509_ENHANCED_KEY_USAGE` 的 policy OID sequence；当前 SDK还列出 auto-update CA/end revocation、no-OCSP-failover 等 policy OID。 |
| `CERT_DISALLOWED_ENHKEY_USAGE_PROP_ID` | `122` / `.122` | 仅针对指定 EKU 的 disallowed 语义。 |
| `CERT_NOT_BEFORE_FILETIME_PROP_ID` | `126` / `.126` | NotBefore 时间限制；不是证书 intrinsic `NotBefore` 的替代品。 |
| `CERT_NOT_BEFORE_ENHKEY_USAGE_PROP_ID` | `127` / `.127` | NotBefore 与 EKU 的组合限制。 |
| `CERT_DISALLOWED_CA_FILETIME_PROP_ID` | `128` / `.128` | CA disallowed 时间属性。 |

这些是**证书 context property / CTL entry attribute 的编号**，不应被误写成公开保证的“AuthRoot 扩展 OID 语义完整规范”。其中 `.105` 的编码方式有 SDK 注释支持；其他属性的生效组合、版本兼容、与 Disallowed CTL 的优先级必须用 Microsoft API 和 fixture 逐项验证。

### 不应猜测的 policy 行为

以下做法不能作为生产修复：

- 看到一个 entry 就忽略未知 attribute 或 critical extension；
- 只检查证书 intrinsic `NotBefore`/`NotAfter`，忽略 root-program `NotBefore`、EKU removal、Disable、Remove 和 Disallowed；
- 只看 `CERT_ROOT_PROGRAM_CERT_POLICIES_PROP_ID` 是否存在，不解码其 policy sequence；
- 把 `CERT_ROOT_PROGRAM_NAME_CONSTRAINTS_PROP_ID` 当作“没有约束”；
- 把 Disallowed CTL 当作可选的黑名单；
- 以当前系统 `Cert:\LocalMachine\Root` 或 logical AuthRoot 中存在为理由，跳过 CTL 签名、sequence、freshness 和 policy。

Microsoft 的 [root deprecation 语义](https://learn.microsoft.com/en-us/security/trusted-root/deprecation)明确区分 Remove、Disable、EKU Removal 和 NotBefore。Aura 若没有能力对这些状态做出同等决定，应该把该 entry 标为 `UnsupportedPolicy`，让 DoH/DoT strict 路径失败，而不是扩大信任。

## 哪些 API 会网络或写入系统

### 明确不放进 Runtime/Profile 路径的调用

- `certutil -syncWithWU`、`-f` 或任何用于获取 CAB/CRT 的同步命令；
- AuthRoot/Disallowed auto-update provider；
- `CertVerifyCTLUsage` 的默认更新路径；
- AIA/URL retrieval helper；
- registry system store 的可写打开方式；
- 全局配置 `HKLM`/Internet retrieval policy 的修改。

`CertVerifyCTLUsage` 尤其不能被简化为“离线 API”：Microsoft 文档说明，如果没有 `CERT_VERIFY_INHIBIT_CTL_UPDATE_FLAG`，过期 CTL 可能被新 CTL 替换，并且 signer info/`NextUpdateLocation` 等可提供更新位置；默认 signer 搜索也不等于 pinned signer。即使设置了 inhibit flag，Aura 仍应使用显式 memory stores、预先读取的 bytes 和 URL retrieval 禁止策略，并对结果做自己的 freshness/policy gate。因此安全核心使用 detached verification 更容易证明。

### 可用于诊断或受控 materialization 的 CryptoAPI 组合

如果必须用 CryptoAPI 构建诊断 chain，建议：

```text
CERT_STORE_PROV_MEMORY signer_store
CERT_STORE_PROV_MEMORY ctl_store
CERT_CHAIN_ENGINE_CONFIG {
    hExclusiveRoot = selected_root_store,
    hRestrictedRoot = selected_root_store,
    hRestrictedTrust = selected_trust_store,
    hRestrictedOther = supplied_intermediates,
    dwUrlRetrievalTimeout = 0,
    dwFlags = CERT_CHAIN_DISABLE_AIA
}
CertGetCertificateChain(...,
    CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY |
    CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL |
    CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE)
```

实际 C/C++ 实现还必须按目标 SDK 核对结构字段和 flag 组合；上面是边界示意，不是可直接复制的编译片段。`CERT_CHAIN_EXCLUSIVE_ENABLE_CA_FLAG` 不应设置为“方便通过”：Microsoft 文档指出它会把非 self-signed CA 当成 anchor 并跳过相应验证，这不符合根锚点和 strict revocation 的目标。

这组 flags 的含义是“只在允许的本地材料里查找”。`CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY` 不是“revocation 已完成”的证明：cache 未命中或过期仍必须返回 unknown/error，不能用 `CERT_REVOCATION_CHECK_END_CERT`、`CERT_CHAIN_REVOCATION_CHECK_END_CERT`、`CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY` 的组合掩盖缺口，也不能使用 `CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY` 后把 Unknown 当作 success。

### memory store 的写入边界

`CERT_STORE_PROV_MEMORY` 只存在于进程内，添加证书/CTL 不会自动写 system registry。`CertCreateCTLContext` 更适合先解析输入 bytes；如需索引，可以把经过验证的 context 加入 memory CTL store。避免 `CERT_STORE_PROV_SYSTEM_REGISTRY_W` 的可写打开，以保持 Host-transparent 约束。任何落盘 snapshot 都应写入 Aura 自己的版本化目录，并用原子替换和 hash manifest，不能写入 Windows trust store。

## 可实现的最小方案

### Stage 0：离线材料交付

由安装/维护工具或开发机显式提供：

- AuthRoot CTL bytes（优先保存原始 `authroot.stl`，同时保存 CAB/hash metadata）；
- 对被选 entry 的 `.crt` DER；
- Disallowed CTL bytes；
- 有效 signer cert chain/允许 signer SPKI hash；
- 需要的 intermediate 和 CRL/OCSP snapshot，以及每份材料的 SHA-256、来源、获取时间、适用 Windows/SDK 版本。

这个阶段可以用 Microsoft 文档描述的 `certutil -syncWithWU` 在有网络的维护环境生成材料，再把生成物带到离线运行环境。Runtime 不执行同步，也不依赖 Host DNS/代理/PAC。

### Stage 1：解析和 detached signer verification

1. 对 CTL bytes 调用 `CertCreateCTLContext(X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, ...)`；输入损坏、编码不完整或 context 缺字段立即失败。
2. 从 `CTL_CONTEXT` 取得 `pCtlInfo` 与 `hCryptMsg`；检查预期 list usage/`SubjectAlgorithm`/`ListIdentifier`。
3. 用只含 pinned signer 的 memory store 调 `CryptMsgGetAndVerifySigner`，显式使用 `CMSG_TRUSTED_SIGNER_FLAG`。如 CTL 有多个 signer，按 index 验证并要求所有实际 signer 都在允许集合中，不能只接受第一个成功的 signer。
4. 对返回 signer 证书检查 DER/SPKI hash、签名链、有效期、允许算法和 `szOID_ROOT_LIST_SIGNER` usage。未知 signer、未覆盖轮换、弱算法或缺少用途都失败。
5. 对 Disallowed CTL 重复同样的签名、signer、freshness 和 list identity gate；不能只验证 AuthRoot 而跳过黑名单。

### Stage 2：freshness 和 anti-rollback

校验 `ThisUpdate`/`NextUpdate`、sequence、list identity、允许的时钟偏差和最大离线窗口。使用已接受 snapshot 的 `(ListIdentifier, SequenceNumber)` 拒绝回滚；保存输入 hash 和 signer fingerprint。`NextUpdate` 缺失、已过期、远未来、时间字段溢出或比较结果不确定都失败。

### Stage 3：entry policy materialization

对每个候选根：

1. 通过 `SubjectIdentifier` 查找精确 DER，不依赖 subject name 或文件名；验证 hash 算法来自 CTL；
2. 将 entry attributes 按 SDK 支持的 property OID 解码。支持的 property 集合必须版本化；未知/critical attribute、未知 policy OID、无法应用的 name constraints 或 chain policy 都使该根不可用；
3. 应用 intrinsic X.509 `NotBefore`/`NotAfter`、Basic Constraints CA、Key Usage/EKU；再应用 Root Program NotBefore、EKU removal、Disable/Remove、Disallowed；
4. 只为需要的 TLS server-auth 用途输出选定根。根的“存在”不等于对所有用途开放；
5. 保留原始 entry/证书 bytes、解码结果和拒绝原因，方便审计和升级。

### Stage 4：strict TLS 使用

对于 Rustls/WebPKI，Stage 3 只输出经过 gate 的根 DER 和必要 intermediate，不把 Windows logical Root 或 AuthRoot provider 整体转换成 RootCertStore。`RootCertStore` 本身不等于撤销检查；现有 Rustls verifier 必须另有已测试的 CRL/OCSP/cache-only gate，否则 P3 不能宣称 strict revocation 已通过。对于 CryptoAPI 诊断，使用上一节的 restricted chain engine 和 cache-only revocation flags。

上游连接顺序（DoH/DoT/TCP/UDP）可以由 Profile 配置，但每条 TLS 上游都必须复用同一 trust snapshot。所有上游失败时返回 typed DNS failure；绝不调用 Windows Host DNS 作为 fallback。AuthRoot verifier 失败时也不能降级到“系统证书 + 在线更新”。

### Stage 5：发布和轮换

每个 snapshot 发布一份 manifest：

```text
schema_version
windows_sdk_or_policy_revision
ctl_sha256
ctl_list_identifier
ctl_sequence_number
ctl_this_update
ctl_next_update
ctl_signer_spki_sha256[]
selected_root_der_sha256[]
disallowed_ctl_sha256
crl_ocsp_snapshot_sha256[]
created_at / expires_at
```

新 signer、未知 policy 或 CTL schema 变化先进入研究/fixture；不能因为 Windows 更新后出现新属性就自动扩大信任。轮换失败时旧 snapshot 只有在自己的 `NextUpdate` 和 revocation window 内可继续使用，超过期限则 strict 失败。

## 当前问题的分步修复建议

| 阶段 | 交付 | 当前可否完成 | 门禁 |
| --- | --- | --- | --- |
| P0 | 仅实现/测试 detached CTL parser、signer store、freshness 和 anti-rollback；全部使用签名 fixture | 可以，不需要 VM；不改生产路径 | 错误 CTL、错误 signer、过期/回滚/未知 attribute 全部拒绝 |
| P1 | 由维护环境采集当前 AuthRoot/Disallowed CTL、signer DER、Cloudflare/Google 目标根和中间证书，生成 hash manifest | 可以，但需要一次受控材料采集 | 每个材料的来源、hash、Windows build 和 signer 轮换证据齐全 |
| P2 | 针对本机当前 CTL 解析 `.83/.84/.104/.105/.122/.126/.127/.128`，把支持集固定到 schema/version fixture | 可以先做只读工具；完整跨版本语义仍需 Microsoft API/多版本验证 | 未知属性不自动放行；NotBefore/EKU/Disallowed matrix 完整 |
| P3 | 用选定 root 构建 DoH/DoT Rustls trust snapshot | 部分可以；已验证的读取来源尚不足以通过公共链 | Cloudflare/Google IPv4 TLS 在 strict revocation 下成功，且没有 Host fallback |
| P4 | 建立离线 CRL/OCSP 快照和过期/unknown 状态 | 先检查现有只读 Cryptnet cache；仍缺项再规划受控采集，VM 不是必要条件 | 缺失、过期、未知撤销始终失败；不在线取回 |
| P5 | 双架构/重启/断网/IPv6 disabled/Host DNS sink 验收 | 需要真实 Windows 环境；现有机器可先做部分 | x64/x86 一致，Host DNS 零流量证据独立记录 |

当前最小可落地工作顺序是 P0 → P1 → P2，再决定 P3/P4 是否有足够离线撤销材料。不要先把 P3 写成“AuthRoot 全量导入”；那会把 `UnknownIssuer` 换成 `UnknownRevocationStatus`，却没有解决 strict trust 的根因。

后续同日增量：CA store 与用户 Cryptnet cache 是不同来源。本轮只读列表显示约 80 个 CRL cache 条目，不能把早期 CA store 观察推广为“本机没有材料”。新增固定 cache-only/no-write flags 读取后，双架构 Google presented chain 各命中 2 份未经认证的 CRL 候选，Cloudflare 为 0；研究 AuthRoot 变体的标准严格验证仍为 `UnknownRevocationStatus`。未解析出精确缺失链节点，不从条目数量或 cache hit 推断全链吊销验证通过。见 [缓存实现与证据](doh-offline-crl-cache.md)。

## 明确不能证明的部分

1. **没有官方公开的完整跨版本 AuthRoot policy wire schema。** Windows SDK 的 property id 和局部注释能支持识别/解码入口，但不足以证明所有属性组合、优先级、版本行为；实现不应自行推导未记录语义。
2. **没有一个可永久硬编码的 Microsoft AuthRoot signer thumbprint。** signer 可能轮换；必须把 signer 材料和轮换作为受控输入，并通过 `CMSG_TRUSTED_SIGNER_FLAG` 隔离系统其他证书。
3. **`CertVerifyCTLUsage` 不是自动的纯离线保证。** 不抑制更新时可能尝试替换过期 CTL；即使显式 inhibit，也要自己限制 signer store、URL retrieval 和输入来源。detached verification 更容易做无网络证明。
4. **`CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY` 不等于撤销成功。** 它只禁止网络取回并依赖现有 cache；cache 不完整、过期或 unknown 必须 fail closed。
5. **AuthRoot CTL 不携带目标链完整撤销材料。** root entry 通过不意味着 intermediate 的 CRL/OCSP 已可离线验证；当前本机实验正是从 unknown issuer 进入 unknown revocation。
6. **本机 `certutil` 成功不等于 Aura snapshot 成功。** `certutil` 验证的是 crypt32 管理的 AuthRoot 材料；Aura 的 Rustls trust store、选根策略和离线撤销必须分别验证。
7. **本记录没有证明 IPv6。** 当前用户已关闭 IPv6；本研究不修改宿主设置，也不把 IPv6 的 network failure 当作 TLS 结果。
8. **本记录没有证明 WFP/应用自带 DoH/DoT/DoQ 的全机隔离。** AuthRoot 只处理 TLS trust material，不替代网络层强制路由。

## 通过标准

AuthRoot 离线支持只有在以下证据都具备时才能从 research 提升为 production candidate：

- 同一份 CTL 在 x64/x86 上由显式 signer store 验签成功，未知 signer/属性/算法失败；
- `ThisUpdate`/`NextUpdate`/sequence/list identity 的正反 fixture 完整，且重复运行不触发网络或 registry 写入；
- 选定 root 的 DER 与 CTL entry 精确匹配，Root Program policy、NotBefore/EKU、Disallowed 和证书 intrinsic constraints 均有逐项测试；
- 断网状态下 DoH/DoT 的公共链或受控 test chain 能在有足够离线 CRL/OCSP 时严格通过；材料缺失/过期/unknown 均严格失败；
- 默认 Windows ROOT、logical AuthRoot、AIA、WinInet/系统代理和 Host DNS 均没有被 Runtime 修改；
- AuthRoot/Disallowed 更新、signer 轮换或 policy schema 变化会产生明确的 `UnsupportedPolicy`/`TrustSnapshotExpired`，不会静默放宽；
- 目标 Windows build、Rust 1.99、x64/x86 以及 cold-start/restart/IPv6-disabled 的证据分别留存。

在这些条件满足前，产品策略应保持：只使用已验证的受控信任快照；AuthRoot 缺材料或策略无法解释时返回明确 TLS/DNS failure；不调用 Host DNS、不在线补证书/CRL、不放宽 revocation。

## 本机 SDK 证据定位

以下不是网络来源，而是当前构建机的 Windows SDK 观察，版本为 `10.0.26100.0`：

```text
C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um\wincrypt.h

3464-3465  szOID_ROOT_LIST_SIGNER = 1.3.6.1.4.1.311.10.3.9
3548-3549  szOID_DISALLOWED_LIST = 1.3.6.1.4.1.311.10.3.30
9299-9300  CERT_ROOT_PROGRAM_CERT_POLICIES_PROP_ID = 83
            CERT_ROOT_PROGRAM_NAME_CONSTRAINTS_PROP_ID = 84
9333-9334  CERT_DISALLOWED_FILETIME_PROP_ID = 104
            CERT_ROOT_PROGRAM_CHAIN_POLICIES_PROP_ID = 105
9365-9372  CERT_DISALLOWED_ENHKEY_USAGE_PROP_ID = 122
            CERT_NOT_BEFORE_FILETIME_PROP_ID = 126
            CERT_NOT_BEFORE_ENHKEY_USAGE_PROP_ID = 127
            CERT_DISALLOWED_CA_FILETIME_PROP_ID = 128
9433-9438  szOID_CERT_PROP_ID_PREFIX = 1.3.6.1.4.1.311.10.11.
9500-9503  chain policies encoded as X509_ENHANCED_KEY_USAGE
9989-10002 AuthRoot EncodedCtl/authroot.stl/authrootstl.cab/authrootseq.txt
```

SDK 注释是实现和 fixture 的输入，不是对未来所有 Windows build 的无限兼容承诺。任何升级后的新增 symbol/attribute 都应先进入研究门禁。
