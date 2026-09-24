# EnvBox V0.2 路线图（规划）

**Status:** draft  
**前置:** V0.1 已冻结 — 真实应用（Mimo 客户端 + 子进程 pwsh）验证核心链路成立。

## V0.1 冻结结论

| 不变量 | 实证 |
|--------|------|
| Process-scoped | 仅注入进程树可见 Profile；Host 配置不变 |
| Environment-consistent | Geo / Locale / Language / Timezone 同源 |
| Virtual timezone, real timeline | 时区 Bias/本地换算虚拟；UTC/FILETIME 真实 |
| Host-transparent | 文件系统 / PATH / 系统目录 / 绝对时间保持 Host |
| 非安全边界 | README + About 已声明 |

**明确不做（防范围膨胀）:** 字体、WebRTC、硬件指纹、防检测、完整 Registry Sandbox、WFP Driver、VM 级隔离。

## 目标一句话

> Run Windows applications with isolated locale, region, timezone and network profiles — without a VM.

下一阶段重心：**覆盖率、一致性、使用体验** — 不再证明「能不能做」。

---

## 优先级 1 — Audit Mode（最高价值）

**问题:** 不知道目标程序（Claude / Codex / Mimo）实际读了哪些地域相关 API。  
**交付:**

- Profile 或 CLI 开关 `audit = on`（默认 off，零开销）
- Runtime Hook 在 Fail Open 原语义下**旁路记录**：API 名、tid、tick、是否命中虚拟化、原值摘要/新值摘要
- 输出：`%LOCALAPPDATA%\EnvBox\audit\<instance_id>.jsonl`（进程树可关联 pid/ppid）
- CLI：`envbox audit show <instance_id>` / `envbox audit export`
- Probe 验收：开关 on 时文件有记录且 API 行为与 off 一致；off 时无写盘

**价值:** 直接回答「代理客户端查了什么」；指导后续 Hook 覆盖缺口。

**Tickets 草案:** 20 audit schema + sink；21 hook 旁路记录；22 CLI 查询；23 Probe/验收对比。

---

## 优先级 2 — DNS View → 真实 per-process routing

**问题:** 现在只改「看到的 DNS 配置」（GetNetworkParams 等），解析仍可能走 Host DNS，出现「显示 1.1.1.1、实际走宿主解析」不一致。  
**交付（V0.2 取最小闭环）:**

- 方案 A（首选）: **per-process DNS 客户端路由** — 对 `DnsQuery_*` / `getaddrinfo` 等解析入口按 Profile `VirtualView` servers 转发或指定解析器；不引入驱动
- 方案 B（备选）: 文档化不一致 + Audit Mode 观测；完整 WFP/驱动留 V0.3
- 验收: Profile 指定私有 resolver 时，目标进程解析结果与 Host 默认解析**可区分**；Host 解析行为不变

**边界:** 不做透明 DNS 劫持 / 不重定向其它进程流量。  
**Tickets 草案:** 24 解析 API Hook 设计；25 实现 + Fail Open；26 对比验收。

---

## 优先级 3 — 边界兼容性

按风险排序，逐项补测试与修复：

| 项 | 风险 | 验收要点 |
|----|------|----------|
| 管理员 / 提权子进程 | 注入或 Job 失败 | Startup Fail Policy 不静默；文档说明 |
| x86 目标 / `envbox-runtime32` | 架构不匹配 | x86 子进程注入链路；V0.1 residual |
| `.cmd` / `.bat` → `node.exe` | wrapper 解析、扩展名 shim | 已有部分覆盖；补多级 wrapper |
| Electron 多进程 | 多级 CreateProcess、utility | 子进程树全部带 Profile |
| 异常退出 / Job Object | 崩溃、Terminate、句柄 | `KILL_ON_JOB_CLOSE`；无泄漏；状态→Exited/Failed |
| `caller_requested_suspended` | 调用方要挂起 | 注入后不 Resume（现 residual，补自动化） |

**Tickets 草案:** 30–35，每项独立可测。

---

## 优先级 4 — 使用体验（最后）

- GUI/快捷方式/Run With 打磨（含 Fowler 残留：main.rs 拆分、save/delete 去重）
- Application 列表持久选中态、错误提示
- README 产品定位句（见下）
- 启动延迟理想 &lt;100ms（release 构建复测）

---

## 文案

README 副标题（产品向）:

> Run Windows applications with isolated locale, region, timezone and network profiles — without a VM.

技术说明段保留现有 Process-scoped 表述。

## 执行约定（沿用 V0.1）

- 每票：TDD → `cargo test` → 双轴 `/code-review` → 修 hard/wrong → commit
- 领域词汇以 `docs/CONTEXT.md` 为准；DnsMode 仅 `Host` / `VirtualView`
- Injection 失败拒绝启动；Profile per-instance immutable；Hook Fail Open
- 不反检测、不改 Host、不扩大到上表「明确不做」
