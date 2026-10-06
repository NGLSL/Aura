# Profile 内 DoH bootstrap

域名 DoH 的连接 IP 可以留空。Runtime 在原查询的 deadline/cancel 范围内，通过同一 Profile 中可直接连接的上游解析 DoH authority 的 A 记录，再使用原 URL 发起 DoH。不会调用宿主 DNS、添加默认服务器或修改 TLS 身份。

种子包括 UDP、TCP、DoT、已配置连接 IP 的 DoH、URL authority 为 literal IP 的 DoH。所有连接 IP 留空的域名 DoH 均跳过，避免互相递归。只有这类上游时配置仍可保存，但查询返回失败。IPv6 连接地址自动发现后置。

Bootstrap 复用 Runtime 的 DNS packet、TXID/question 验证和 Windows native record decoder，只接收 answer section 中与请求名或合法 CNAME 链匹配的 A 地址。别名最多 8 跳，拒绝循环、冲突 CNAME、无关 owner 的 A、unspecified/multicast/broadcast 地址。此路径不缓存连接 IP，每次均重新查询，因此不会复用已经过期的 bootstrap 结果。

## 可重复的 native 契约

```powershell
cmake -S tools/envbox-dns-bootstrap-fixture -B target/dns-bootstrap-contracts64 -G "Visual Studio 17 2022" -A x64
cmake --build target/dns-bootstrap-contracts64 --config Release
target/dns-bootstrap-contracts64/Release/envbox-dns-bootstrap-fixture.exe
cmake -S tools/envbox-dns-bootstrap-fixture -B target/dns-bootstrap-contracts32 -G "Visual Studio 17 2022" -A Win32
cmake --build target/dns-bootstrap-contracts32 --config Release
target/dns-bootstrap-contracts32/Release/envbox-dns-bootstrap-fixture.exe
```

这 18 组条件执行真实 `hooks_dns.cpp` 路由、DNS 编解码和 DoH wrapper，以 transport seam 控制响应。覆盖 5 种种子、单包/跨包 CNAME、无关 A、错误 TXID/question、循环、3 类非法 A、无种子、取消、原 deadline，以及 `getaddrinfo` 与 HTTPS QTYPE 65 路由。Host resolver trap/audit 计数必须为 0。它不验证传输实现或真实 TLS。

## 实际注入与 DoH

```powershell
python tools/envbox-dns-bootstrap-fixture/run_injected.py `
  --cli target/debug/envbox.exe `
  --probe64 target/debug/envbox-probe.exe `
  --probe32 target/i686-pc-windows-msvc/debug/envbox-probe.exe `
  --runtime-dir target/doh-profile-bootstrap-runtime
```

Runner 拥有独立配置目录和 loopback UDP/TCP listener。listener 只回答 `cloudflare-dns.com A → alias.bootstrap.invalid A → 1.1.1.1`，不回答应用查询；因此成功不能来自业务查询回退到种子。DoH 使用未修改的 `https://cloudflare-dns.com/dns-query`，执行产品默认证书/TLS验证，不安装信任或修改宿主 DNS。

2026-10-07 实际结果：

- 双架构 native 契约各通过，Host resolver calls=0。
- 新 Runtime `target/doh-profile-bootstrap-runtime`：8/8 注入用例通过，证据 `target/dns-profile-bootstrap-6aae77b836ad4439bd3a807f88c9fdf0/result.json`。每架构包括 UDP bootstrap + DNS65、UDP bootstrap + getaddrinfo、TCP bootstrap + DNS65、只有空 bootstrap DoH 的明确失败。Controller 未载入 Runtime，Host fallback 审计为 0。
- 使用旧 `target/profile-identity-runtime-pair-final` 的同脚本 RED 对照：6 个新 bootstrap 成功场景全部失败，2 个无种子的失败边界保持符合预期；证据 `target/dns-profile-bootstrap-fbeb4b9e43b24b54aa54022c559d1856/result.json`，exit 1。没有覆盖旧结果。

公共 TLS 成功是当时服务行为证据；audit 是产品覆盖路径的观测，不是独立全局 DNS 流量捕获。实注入覆盖 UDP/TCP 种子；DoT、显式/literal DoH 种子的本轮验证为 native route 契约，不扩大为新实网资格。
