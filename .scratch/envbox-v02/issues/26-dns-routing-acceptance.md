Parent: .scratch/envbox-v02/spec.md

# 26: DNS routing — 对比验收

**What to build:** VirtualView vs Host 解析结果可区分；Host 解析行为不变。

**Blocked by:** 25 DNS routing — 实现 + Fail Open

**Status:** resolved

- [x] Host 模式解析与 Host 一致
- [x] VirtualView 使用可控 fixture/本地 resolver 或文档化 contrast
- [x] resolver 不可达不永久挂起
- [x] 矩阵跑完后 Host DNS 不变

## Comments

### 2026-09-24 acceptance

**Evidence**

- Probe 扩展：`envbox-probe --resolve <name>` 打印 `=== RESOLVE ===` + `getaddrinfo:` 结果（向后兼容；无 flag 行为不变）。`probe_cli.rs::resolve_flag_prints_getaddrinfo_section`。
- `crates/envbox-cli/tests/cli_dns.rs`（fixture UDP `127.0.0.1:53`，绑定失败 skip）：
  1. `virtual_view_resolves_via_profile_dns_fixture` — `--dns-mode virtual_view --dns 127.0.0.1`，`fixture.test` → `10.99.0.1`
  2. `host_mode_does_not_force_fixture_result` — Host 模式不得强制 fixture 结果
  3. `virtual_view_and_host_resolution_contrast` — VirtualView vs Host 对比可区分
  4. `unreachable_dns_server_does_not_hang` — `192.0.2.1` 有界超时，进程 &lt; 20s 退出
  5. `host_dns_unchanged_after_dns_routing` — 跑完后本机 `localhost` 解析仍正常
- 既有 `cli_run` 全绿；Host DNS View 测试未回归。
- 矩阵/Host 不变由既有 `run_leaves_host_probe_snapshot_unchanged` 与 acceptance matrix 覆盖。
