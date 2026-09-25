# AGENTS.md

## 项目

EnvBox 是 Windows 原生的进程级环境虚拟化启动器。目标程序直接运行在宿主 Windows 上（文件系统、GPU、网络、用户目录、Git/SSH/IDE 均可用），但进程树读取部分系统环境信息时得到 Environment Profile 指定的值。不提供安全边界，不是沙箱。

产品原则：

1. **Process-scoped** — 修改只属于一个 RuntimeInstance 的进程树
2. **Host-transparent** — 不修改 Windows 全局配置
3. **Environment-consistent** — 同一 Profile 下 Locale/Region/Language/Timezone/DNS/Environment 尽量一致

## 目录

- `crates/envbox-app/`：Iced GUI
  - `src/main.rs`：装配（窗口、主题、入口），不含业务
  - `src/app.rs`：EnvBoxApp 状态与 update / save / run
  - `src/views/`：三栏壳与各页渲染（nav / apps / profiles / instances / audit / settings / detail）
  - `src/theme.rs`：色板与控件样式；`src/widgets.rs`：表单原语；`src/message.rs`：UI 消息
- `crates/envbox-core/`：Application / Profile / RuntimeInstance 领域模型
- `crates/envbox-storage/`：TOML 持久化与校验
- `crates/envbox-launcher/`：命令解析、Environment Block、Job Object、注入
- `crates/envbox-cli/`：`envbox run` 等命令行入口
- `runtime/`：C++/Detours Runtime（`envbox-runtime32/64.dll`，按 hooks 分模块）
- `tools/envbox-probe/`：环境探针，验收基准
- `icons/`：应用图标（多尺寸 PNG + icon.ico，规格对齐 Veya）
- `docs/`：领域词汇、ADR、设计稿
- `.scratch/`：本地 Markdown issue tracker（spec 与 tickets）

## 开发规则

- 不扩大 V0.1 范围；GUI 最后做，先验证 Runtime。
- 每增加一组 Hook，必须同步扩展 Probe。
- 不实现反检测、不隐藏 EnvBox、不修改宿主 Locale/Region/Timezone。
- 不调用 `SetDynamicTimeZoneInformation` 修改系统时区。
- Profile 对每个实例 immutable；运行中不热更新。
- Hook 出错优先 Fail Open 到原 Windows API。
- Injection 完全失败则拒绝启动，禁止静默降级。
- Process/Thread Handle 必须 RAII；保存 `GetLastError()`。
- 先 x64，再 x86；不提前做 WFP Driver / 虚拟机 / 完整 Registry Sandbox。
- 领域术语用 `docs/CONTEXT.md`。

## 验证命令

```powershell
cargo build
cargo test
cargo build -p envbox-broker   # envbox-broker.exe（Session Registry / IPC Host）
# runtime 使用 CMake/MSVC 构建 envbox-runtime64.dll 与 envbox-probe
.\target\debug\envbox-probe.exe
.\target\debug\envbox.exe profile list
.\target\debug\envbox.exe app list
.\target\debug\envbox.exe run --profile us .\target\debug\envbox-probe.exe
# --profile accepts UUID or exact name (e.g. us = profile named "us")
```

验收关注：Probe 正常输出、Runtime DLL 载入、Host 系统配置完全不变。

## Agent skills

### Issue tracker

本地 Markdown，位于 `.scratch/<feature>/`。See `docs/agents/issue-tracker.md`.

### Triage labels

canonical 状态名写入 issue 的 `Status:` 行。See `docs/agents/triage-labels.md`.

### Domain docs

single-context。See `docs/agents/domain.md`.
