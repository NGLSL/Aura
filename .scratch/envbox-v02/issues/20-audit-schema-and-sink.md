Parent: .scratch/envbox-v02/spec.md

# 20: Audit Mode — schema + sink

**What to build:** Audit Event 的稳定 JSONL schema 与 Runtime 旁路落盘 sink；默认 off，sink 失败 Fail Open。

**Blocked by:** （无）

**Status:** resolved

- [x] `docs/CONTEXT.md` 增加 Audit Mode / Audit Event / DNS routing 术语
- [x] RuntimeProfile 增加 audit 开关（`ENVBOX_AUDIT=1`，默认 0）
- [x] Application.audit 字段 + CLI `--audit` 覆盖
- [x] Audit Event schema v1：`v,ts_utc,pid,ppid,tid,api,virtualized,summary`（无密钥/文件内容/完整 env）
- [x] sink：`<config_root>/audit/<instance_id>.jsonl`（`ENVBOX_CONFIG_ROOT` 优先，否则 `%LOCALAPPDATA%\EnvBox`）；宽字符路径
- [x] 单文件 per RuntimeInstance；追加写；子进程经 `ENVBOX_AUDIT` + UpsertProfileKeys 继承；无 instance_id 不写
- [x] sink 打开/写失败不影响 Hook（Fail Open）；无 polling / timer
- [x] Rust 侧 schema 往返单测；C++/Rust 线格式一致（summary 可省略）
- [x] 集成：`envbox run --audit` 生成 jsonl（含 `EnvBoxAuditInit`）；默认 off 时不写盘

## Comments

### 2026-09-24 implementation

**Evidence**
- `envbox-core::AuditEvent` schema v1 + 3 unit tests
- `envbox-storage::ConfigStore::{audit_dir,audit_path,ensure_audit_dir}`
- `runtime/src/audit.cpp`：JSONL append、JSON escape、Toolhelp ppid、Fail Open
- `ENVBOX_AUDIT` 贯通：`build_environment_block` / `LaunchRequest.audit` / CLI `--audit` / `UpsertProfileKeys`
- 测试：`run_audit_on_writes_instance_jsonl`、`run_without_audit_flag_writes_no_audit_file`；`cargo test --workspace --exclude envbox-app` 全绿

### 2026-09-24 dual-axis review (general-16 Standards / general-17 Spec) → fixes

**Standards Hard=1**
- `EnvBoxParentPid`：Toolhelp HANDLE 失败保存 `GetLastError`，统一 CloseHandle

**Standards Judgement 采纳**
- `api` JSON escape；C++/Rust 线格式对齐；SRWLOCK 防并发写交错
- ppid 延迟到首事件（避开 DllMain loader lock 上的 Toolhelp）
- 单一开关源：`RuntimeProfile.audit`（`ENVBOX_AUDIT` 精确 `1`）
- `to_json_line` 返回 `Result`，不再静默丢事件

**Spec Wrong=6 / Missing=2**
- `Application.audit` + `build_launch_request` + CLI/GUI 字段
- 禁 `unknown.jsonl` 共享文件；宽字符路径；UTF-8 summary 不替换为 `?`
- 过长 `ENVBOX_CONFIG_ROOT` 不静默改道 LOCALAPPDATA
- `ENVBOX_AUDIT` 仅精确 `1` 生效

**Residuals**
- `triage-labels.md` 与 `issue-tracker.md` 的 `Status:` 合法值不一致（沿用 V0.1 `resolved`）
- Profile 级 audit 字段未做（Application 级 + CLI 覆盖已够 V0.2）
- `ConfigStore::audit_path` 留给 ticket 22 CLI
