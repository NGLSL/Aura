# Profile DNS transports 与严格路由

Date: 2026-10-04

## 目标与边界

支持有序 UDP、TCP、DoT、DoH 上游；所有 QTYPE 共用报文与传输路径。`strict = true` 时，受支持的 Windows 解析 API 不调用 Host DNS，全部上游失败返回解析错误。协议类型不决定是否 strict；strict 也不等于必须加密。

用户已授权完整 Container P0–P8 的实施，包括本文的 typed upstream、DoT、DoH 与 strict；后续确认无测试虚拟机，先完成可独立实现的部分。实现与验收进度见 `.scratch/aura-container/evidence/implementation-progress.md`，授权不等于各项能力已经交付。本文取代 `.scratch/envbox-v02/issues/24-dns-routing-design.md` 中“其他 type / 上游失败直接 Fail Open、DoH 非目标”的后续设计限制；未支持的地址查询和异步入口必须明确拒绝或独立验收后支持。

范围是 Aura 注入进程的 Windows resolver 路由，不是网络安全边界。应用自带 UDP/TCP/DoH/DoT/DoQ 和未拦截的新 API 仍可能绕过；本文不引入 WFP 驱动或透明流量重定向。

## 当前实现约束

- `crates/envbox-core/src/lib.rs`：`DnsProfile { mode, servers: Vec<IpAddr> }`，无协议、端口、TLS 身份和 strict。
- CLI `--dns` 和 GUI 的 DNS 文本编辑器只接受 IP；保留 `--dns IP` 作为 UDP/53 简写，GUI 后续改成可排序的上游行。
- Launcher 环境回退使用 `ENVBOX_DNS_MODE/SERVERS`；Broker bootstrap 使用重复 `dns_server`；Runtime 固定最多 8 个 64-byte 地址。不可把 URL 塞入旧 IP 字段。
- IPC 行协议上限 8192 bytes，配置解析存在静默过滤地址；Runtime 环境配置溢出存在切回 Host 的路径。strict 下这些情况必须报错或拒绝启动。
- Broker 当前负责 Session Registry / Profile bootstrap。目标进程在 Aura 退出后仍须继续运行，不把 DNS 数据面依赖绑到 GUI 或现有 Broker 寿命。

## 配置模型

项目使用 TOML。新增 `upstreams` tagged enum，顺序即实际尝试顺序，不自动补入未配置的公共 DNS 或明文上游。示例：

```toml
[dns]
mode = "virtual_view"
strict = true

[[dns.upstreams]]
type = "doh"
url = "https://cloudflare-dns.com/dns-query"
bootstrap_ips = ["1.1.1.1", "1.0.0.1"]

[[dns.upstreams]]
type = "dot"
address = "1.1.1.1"
port = 853
server_name = "cloudflare-dns.com"

[[dns.upstreams]]
type = "tcp"
address = "1.1.1.1"
port = 53

[[dns.upstreams]]
type = "udp"
address = "1.1.1.1"
port = 53
```

这是明确允许加密失败后使用明文的配置。仅希望加密的 Profile 只配置 DoH/DoT；`strict=true` 只禁止 Host fallback，不能替代加密策略。GUI 显示实际上游顺序和是否存在明文 fallback，不增加隐藏协议优先级。

- UDP/TCP/DoT 的 `address` 必须是 literal IP；UDP/TCP 默认 53、DoT 默认 853。端口范围 1–65535。
- DoT 的 `server_name` 是证书校验身份和适用时的 SNI，不从宿主解析。支持 IP 身份时必须验证证书 IP SAN，不能关闭证书校验。
- DoH 仅允许 `https://`，保留 URL 的主机名作为 HTTP authority、SNI、证书身份。主机名 URL 必须显式配置 `bootstrap_ips`；IP URL 可直接连接该 IP 并按 IP 身份验证证书。
- 首版不自动解析上游域名、不跟随重定向、不继承系统代理/PAC/凭据。TLS 至少 1.2；证书有效期、信任链、主机名错误均返回失败。
- 旧 `servers=[IP...]` 按原顺序迁移为 UDP/53；不得自动改为 Cloudflare 或其他加密服务。不允许 `servers` 与 `upstreams` 同时生效。
- 新建 VirtualView 默认 strict。旧 VirtualView 迁移也使用 strict 并在升级说明中说明失败行为改变；确需兼容 Host 回退的用户显式设置 `strict=false`。Host 模式仍是单独的显式选择。

## 分层与接口

保留 Rust 配置控制面与 C++ Runtime 数据面。把 `hooks_dns.cpp` 的 packet/transport/record 部分拆到边界明确的模块，所有解析入口调用同一个 Query Engine。

```text
Windows API adapter
  -> Query Engine (QNAME/QTYPE/QCLASS/options, CNAME, negative result)
     -> Ordered Upstream Router (strict, total deadline, cancellation)
        -> UDP / TCP / DoT / DoH transport
     -> DNS response validation + Windows record conversion
  -> DNS_RECORD / addrinfo / async callback
```

Transport 最小接口是 `exchange(upstream, dns_packet, deadline, cancel) -> response_packet | error`。协议层不按 QTYPE 分支。UDP 截断时向同一 IP/端口 TCP 重试，共享预算；加密上游失败仅进入 Profile 显式配置的下一项。

统一检查响应来源/事务 ID/QR/opcode/Question QNAME/QTYPE/QCLASS，处理压缩、CNAME 有界追链、NXDOMAIN/NODATA、最大报文大小与畸形输入。NXDOMAIN/NODATA 是最终结果；连接错误、超时、SERVFAIL/REFUSED 和不可用上游进入下一项。TLS 身份失败只能尝试配置中下一项，不忽略证书错误重试。

## DoT 与 DoH 实现决策

DoT 使用连接到 literal IP 的 socket + Schannel，在 TLS 内使用 DNS TCP 的两字节长度前缀；连接复用按 immutable Profile / upstream 身份隔离。依据 [RFC 7858](https://www.rfc-editor.org/rfc/rfc7858.html)。

DoH 首选系统 WinHTTP，以 HTTPS POST 发送原始 DNS wire packet，`Content-Type` 与 `Accept` 为 `application/dns-message`；校验 HTTP 状态、内容类型、长度和 DNS 响应，启用 HTTP/2 能力。依据 [RFC 8484](https://www.rfc-editor.org/rfc/rfc8484.html)。

DoH 选型前必须完成真实 Windows 原型：URL 保留 DNS 名称，使用 literal bootstrap IP 连接，验证 Host/SNI/证书仍使用 URL 名称，且零 Host DNS 请求。微软提供 [WINHTTP_OPTION_RESOLUTION_HOSTNAME](https://learn.microsoft.com/en-us/windows/win32/winhttp/option-flags) 改变解析目标，但文档本身不能证明所有目标 Windows 上 IP override、TLS 身份与 HTTP/2 组合均符合要求。用 `WINHTTP_OPTION_CONNECTION_INFO` 核对实际远端 IP，抓取专用夹具日志验证 bootstrap 没有旁路。

原型不达标时返回技术选型问题，不退回宿主 bootstrap，也不临时手写完整 HTTP/2。再评估成熟的进程内 HTTP/TLS 库与 x86/x64 打包成本。DoT 不因这个选型门禁被阻塞。

证书链构建/吊销检查也可能产生辅助网络请求；严格验收必须观测它们。首次实现使用本机信任材料和不产生隐式 DNS 的校验路径，并明确吊销检查策略，不能用跳过证书校验解决泄漏。

## Strict 契约

- 统一收紧 `DnsQuery_A/W/UTF8/Ex`、`getaddrinfo`、`GetAddrInfoW`、`GetAddrInfoExA/W` 的成功/失败/异步路径。非 ASCII 名称显式 IDNA 或返回不支持；不透传网络查询。
- 不支持的 query options/version/interface/caller server list 在 strict 下明确报错。纯本地 numeric/localhost 操作可以本地完成；强制 wire 选项不得借本地直通触发 Host DNS。
- 启动时验证关键 Hook、配置版本、Profile 完整性和能力。strict 必需能力缺失拒绝启动；不能以 Hook 安装失败或环境回退为理由变成 Host。
- Broker 不可用时用完整版本化 snapshot 重建相同上游/strict；重建失败报错。子进程保持 immutable Profile。
- 单次请求全部上游、TLS、HTTP、TCP重试共享总 deadline；取消后不尝试下一上游，回调至多一次，所有资源关闭。连接池有容量和空闲生命周期边界。
- 暂不新增任意 RR 缓存；已有地址缓存只允许同一 immutable Profile 内使用。后续缓存必须遵守实际 TTL，key 包含 Profile 配置身份、QNAME/QTYPE/QCLASS 与影响语义的选项。
- 审计记录 transport、上游配置序号、耗时、错误类别、fallback 和 strict-block。默认不记录完整查询域名或 DoH URL 的敏感参数。
- 现有 Network Guard DNS allowlist 同步检查 UDP/TCP/853/443 和 bootstrap 地址，保证内部 transport 不被自身策略误拦；不因此宣称应用自带解析器无法绕过。

## 实施顺序与验收

1. **完成当前 DNS 修复**：全 QTYPE DnsQuery、受支持的同步地址查询在 Profile 失败时不回退 Host、UDP/TCP、原生记录转换、CNAME、取消。真实注入夹具对比旧 DLL 与新 DLL；保留尚未支持的地址查询输入与异步入口的明确说明。
2. **统一 Query Engine 和 strict**：所有既有解析入口共用路由；完整 Profile/IPC/env snapshot、Hook 能力门禁。实际超时/失败时 Host DNS 夹具零请求；Host 模式与显式 non-strict 单独验证。
3. **Typed upstream 配置贯穿全链**：Core/Storage/CLI/GUI/Broker/Runtime 同步，旧配置迁移、保存 roundtrip、排序、错误校验和 x86/x64 DTO。配置损坏、超限、字段丢失不得切 Host。
4. **DoT**：本地 TLS 夹具验证成功、错误证书、身份不匹配、截断/半包、超时、取消、连接复用和顺序 fallback。
5. **DoH bootstrap 原型与实现**：证明显式 IP + 正确 TLS 身份 + HTTP/2 + 零 Host DNS；随后验证 POST wire、非2xx、错误媒体类型、畸形/超大body、重定向拒绝、代理隔离、取消和总deadline。
6. **四种 transport 集成验收**：每种协议运行 A/AAAA/HTTPS/SVCB/TXT/PTR/SRV/CNAME/NS/root/unknown QTYPE；混合顺序、所有上游失败、仅加密配置、并发与资源稳定、父子进程与 Aura 退出后的持续解析。

完成标准：宿主控制进程未经 Aura 注入；受支持的 Windows API strict 测试零 Host DNS；宿主全局 DNS 设置不变；本地夹具、真实公共服务、x64/x86 构建与实际注入分别报告证据。浏览器自带 DoH 不属于这套 API 拦截验收，应单独报告。
