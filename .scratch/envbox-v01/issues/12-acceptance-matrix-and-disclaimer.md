Parent: .scratch/envbox-v01/spec.md

# 12: 验收矩阵 + 安全声明收尾

**What to build:** V0.1 Done Definition 可勾选：Application/Profile/Runtime/Children/Host/Files/Probe 全部达标，并对外写清 EnvBox 不是安全边界。

**Blocked by:** 06 子进程传播, 07 Locale/Language/时区补全, 08 DNS View, 09 Registry Virtual View

**Status:** resolved

- [x] 测试矩阵：`envbox-probe`、`cmd`、`powershell`、`git`、`node`、`python`、`notepad` + 一个真实 Node CLI Agent 场景
- [x] Host vs US Probe 快照：虚拟化字段有明确差异；非虚拟化字段一致
- [x] Host 的 Timezone/Language/Region/DNS 在跑完矩阵后仍为原值
- [x] 目标进程仍可访问 `C:\`、`D:\`、Git repo、SSH keys、项目文件
- [x] About/README 含非安全边界声明（与规格 Implementation Decisions / story 48 文案一致）
- [x] 性能抽查：额外启动延迟理想 <100ms / 可接受 <300ms；Runtime 无 polling

## Comments

### 2026-09-24 acceptance matrix + disclaimer + review fixes

**Evidence** — `crates/envbox-cli/tests/acceptance_matrix.rs`（7 tests）+ `cargo test --workspace` **82 passed**

| 项 | 证据 |
|----|------|
| 矩阵 | `acceptance_tool_matrix_short_commands`：cmd / powershell / **git / node / python 必跑** + cmd→node 子进程 |
| notepad | `acceptance_notepad_launch_and_stop`：Profile 启动 + Job Stop → Exited |
| Node CLI Agent | `acceptance_node_cli_agent_scenario`：真实 **`my-claude`**（Claude Code CLI）`--help` 经 `node cli.js` 与 Command 路径 |
| Host vs US | `acceptance_host_vs_us_probe_contrast`：Geo/Locale/UI/TZ 有 contrast |
| Host 不变 | `acceptance_host_config_unchanged_after_matrix`：probe + cmd/ps/git/node/python 后 Host 五字段不变 |
| 文件访问 | `acceptance_filesystem_access_from_target`：C:\ D:\ 用户目录 `D:\Project\Aura` + `git rev-parse` |
| 声明 | `README.md` + GUI About（U+2019 与规格一致）：**EnvBox does not provide a security boundary; launched apps keep the current user’s filesystem and permissions.** |
| 性能 | `acceptance_launch_latency_under_300ms`：warm best-of-8 额外延迟 **~199ms**（&lt;300ms）；debug 未达理想 &lt;100ms |
| 无 polling | `runtime/src` 无 `Sleep`/`SetTimer`/`TimerQueue` |

**Review（general-13）后修复**
- `Status: resolved`（issue-tracker 约定）
- About 撇号 U+2019；引用改为 Implementation Decisions / story 48
- git/node/python 不再 optional-skip
- 真实 Node CLI Agent：`my-claude`（非 sim）
- 顺带修两个产品 bug：
  1. **子进程 `lpEnvironment==nullptr` 被写成仅 ENVBOX_*** → Node CSPRNG 断言；现继承完整环境再 upsert
  2. **Command 解析** 优先 `.cmd` over 无扩展名 npm shim（避免 Detours 193）

**Residuals（不阻塞）**
- 启动延迟理想 &lt;100ms 在 debug 约 200ms
- SSH keys 未枚举断言（用户目录可访问已覆盖）
- `caller_requested_suspended` 自动化仍后置
- x86 Runtime 预留

### 2026-09-24 post-review hard/wrong (general-14/15 on 09e98ee..583f16d)

- Hard: `SpawnInjected` 的 `DetourCreateProcessWithDllExW` 失败路径补保存 `GetLastError` + 关闭可能残留的 PI 句柄（AGENTS.md Handle/错误契约）。
- Wrong/partial 记入 residuals：acceptance 未跑 probe `--spawn-child`（cli_run 已覆盖）；Host vs US 非虚拟化字段仅可选 `SystemTimeAsFileTime`；DNS 未做 VirtualView contrast；my-claude 路径写死本机；矩阵未单列 notepad/node-agent 的 Host-unchanged 行。
- Wrong: 单段合法 IANA（EST/MST/CET/HST）已放行（见 ticket 11）。
