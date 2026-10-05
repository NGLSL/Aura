# 票 05：真实入口与真实应用独立验收

Date: 2026-10-06

Status: partial-validation

这份记录是没有测试虚拟机时可以独立完成的真实入口切片。它不关闭票 05，也不宣称完整 Container 目标完成；`Unverified` 入口和浏览器 renderer 缺口仍然保留。

## 运行边界与输入身份

使用仓库根目录的正常外层命令运行，外层通过 `Win32_Process.Create` 创建隐藏 worker：

```powershell
powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File .\tools\envbox-real-app-matrix\run.ps1
```

最终运行时间为 `2026-10-05T18:29:22.0993190Z`（本机 `2026-10-06 02:29:22 +08:00`）。worker PID 为 `2752`，worker 自身加载的 `envbox-runtime*` 模块数为 `0`；这保证宿主对照没有继承当前 Aura Runtime。宿主为 Windows 11 Education build `26200`，PowerShell `5.1.26100.9444`。

本次使用的冻结 Runtime V3 pair 如下。它是本轮可复现的旧验收输入，不能冒称异步 Runtime 或后续构建的最终发布身份。

| 架构 | 实际路径 | SHA-256 |
| --- | --- | --- |
| x64 | `target/container-independent-final-runtime-v3/envbox-runtime64.dll` | `215C42DC101265A45343C8F599ACDD9DE2AABE999578ECF60011F81C54146D36` |
| x86 | `target/container-independent-final-runtime-v3/envbox-runtime32.dll` | `438D56E88E73EB0D2FCA87400EC2F134C937041555C709611BDB2857BE0F7E1A` |

每次运行都生成独立的 `%TEMP%\envbox-real-app-matrix-{GUID}` 根目录、临时 Aura 配置和 Chrome `--user-data-dir`。本次实际浏览器为 Chrome `154.0.8037.97`（`C:\Program Files\Google\Chrome\Application\chrome.exe`）。没有打开用户已有 browser profile，没有加入 `--no-sandbox`，没有安装应用、提权或激活 Packaged 应用。

宿主对照由同一个 fresh worker 启动的未注入 Probe 完成：Runtime marker 数为 `0`，对照读取仍为宿主的 `CN`、`zh-CN`、`Asia/Shanghai`。Profile 进程的保留输出分别显示 `en-US`、`US`、`America/Los_Angeles` 和 `ENVBOX_MATRIX=profile`；这组差异来自真实 Profile 进程输出，未用宿主值推断 Profile 成功。

## 入口结果

状态是每一行独立计算的。`harness_status=completed` 只表示夹具完成并完成清理；JSON 的 `exit_code=0` 是 harness exit code，不是整张矩阵通过。

| 入口 | 状态 | 实际证据与边界 |
| --- | --- | --- |
| CreateProcess + CreateProcessAsUserW | **Verified** | x64 `envbox run` 启动 `envbox-probe --spawn-child --spawn-as-user-child`。保留 stdout 有 3 个 `EnvBox Runtime Loaded`、3 个 `ENVBOX_MATRIX=profile`，并有 `Status: succeeded`；Runtime 启动行在 stderr 中单独确认。目标 PID `22656`，启动 generation/creation stamp `639268217566938064`。wrapper 的 owned handle 立即保存，初始 `GetExitCodeProcess` 为 `259/STILL_ACTIVE`，等待后通过同一 handle 读到真实 exit code `0`，`timed_out=false`。 |
| `cmd.exe` console | **Verified** | 真实 `cmd.exe /d /c echo ENVBOX_MATRIX=%ENVBOX_MATRIX%` 输出 `ENVBOX_MATRIX=profile`，stderr 有 Runtime startup line。wrapper owned handle 的最终 exit code 为 `0`，没有超时。 |
| PowerShell console | **Verified** | 真实 `powershell.exe -NoLogo -NoProfile -NonInteractive` 输出 `profile`，stderr 有 Runtime startup line。wrapper owned handle 的最终 exit code 为 `0`，没有超时。 |
| ShellExecute | **Unsupported** | 真实 `ProcessStartInfo.UseShellExecute=true` 隐藏启动 Probe，输出只有宿主对照值，无 Runtime marker、无 Profile 环境。没有打开现有用户应用；产品没有全局 ShellExecute 拦截承诺。 |
| WMI `Win32_Process.Create` | **Unsupported** | 外层 WMI 确实创建了 fresh worker；worker 内的未注入 Probe 没有 Runtime marker。为避免 WMI provider callback 嵌套等待导致 worker 卡住，夹具没有把嵌套 WMI 伪装成成功的 EnvBox target injection；该入口能力保持产品 Unsupported。 |
| Chromium/Edge root | **Unverified** | Aura Chrome root target PID `7548`（creation stamp `639268217602435126`）被真实启动，stderr 有 `Runtime + core hooks active` 与现有 partial-renderer 提示，但进程树采样没有 root Runtime module，stdout 为空。使用同样 hidden headless 参数和独立临时 profile 的未注入 Host Chrome target PID `17824`（creation stamp `639268217585745804`）也退出为 code `13`；Aura target 同样退出为 code `13`。这个对照说明当前系统/Chrome headless 启动本身失败，不能把结果归因成 Aura Unsupported，也不能据此确认 root Runtime 覆盖。 |
| Chromium/Edge renderer | **NotObserved**（renderer 不得 `Verified`） | Host 与 Aura 两次 Chrome 都在建立 renderer 观测前以 code `13` 退出，没有 renderer PID、Runtime 或 Environment View 证据。renderer 覆盖保持 NotObserved；产品层的 renderer ceiling 仍单独限制为 Partial/Unsupported，不能由 root 或 Host 对照升级。 |
| WithToken | **Unverified** | 没有安全的独立 WithToken fixture。已有的 CreateProcessAsUserW 结果不被推断成 WithToken 支持。 |
| Native `NtCreateUserProcess` | **Unverified** | 没有 Native API fixture；没有用 CreateProcess 或 WMI 结果替代 Native 入口。 |
| Packaged/AUMID | **Unverified** | 没有激活可能显示 UI 或复用已有 singleton 的应用，未枚举或触碰用户已有 packaged app。 |

浏览器的 `single_instance_transfer` 仍为 `Unverified`。独立临时 profile 只证明夹具没有复用已有 singleton，不能证明宿主已有 singleton 转交行为。

## PID generation 与清理证据

Aura 浏览器 target 启动时记录了 PID `7548` 和 creation stamp `639268217602435126`。目标在清理前已经自然退出，因此再次读取到的 actual stamp 为 `null`，结果明确记录为：

```text
stopped: true
reason: root-already-exited; no descendant walk
target_exists_after_stop: false
remaining_tree: []
```

这个分支没有按 PID/PPID 快照递归杀进程。`Stop-OwnedTree` 先用 `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE)` 打开一次稳定句柄，再用同一句柄的 `GetProcessTimes` 转成与 expected 相同的 `DateTime.Ticks` 基准比较；只有精确匹配本次启动 generation 才调用同一句柄的 `TerminateProcess`。creation stamp 缺失或不匹配时拒绝停止，句柄关闭在 finally 中完成。普通超时路径也把同一 expected stamp 传入，wrapper 只由本次 `Start-Process` 创建并由本次夹具拥有。

本次还运行了独立的隐藏 PowerShell safety fixture（PID `17608`，expected stamp `639268217565644212`）：

- 错误 stamp `639268217565644212-wrong` 被拒绝，结果 `stopped=false`，实际同句柄 creation stamp 仍为 `639268217565644212`。
- 错误 stamp 返回后，原 owned handle 仍报告 `259/STILL_ACTIVE`，证明 fixture 仍存活。
- 正确 stamp 使用稳定句柄停止，返回 `terminated; exit confirmed on owned handle`，同一原句柄最终报告 exit code `1`。
- `wrong_stamp_rejected=true`、`live_after_wrong_stamp=true`、`correct_stamp_stopped=true`、`exit_confirmed_on_original_handle=true`，该安全控制状态为 `Verified`。

本次临时根目录清理结果为：

```text
resolved_root: C:\Users\admin\AppData\Local\Temp\envbox-real-app-matrix-31600c32e2834e239a1f4f4a120509ff
contained_by_intended_parent: true
root_was_reparse_point: false
processes_using_root: []
removed: true
residual_paths: []
error: null
```

清理失败不会被吞掉；它会写入 `cleanup.error`，并使 harness `exit_code=2`。删除前还会拒绝任何命令行仍引用该临时根目录的进程。本次运行结束后没有残留 `envbox-real-app-matrix-*` 根目录，也没有现存 `chrome.exe` 命令行引用本次临时 profile。没有终止非本次拥有的进程。

## 可复核产物与限制

脚本位于 [`tools/envbox-real-app-matrix/run.ps1`](../../../tools/envbox-real-app-matrix/run.ps1)，使用说明位于 [`tools/envbox-real-app-matrix/README.md`](../../../tools/envbox-real-app-matrix/README.md)。本次生成的汇总为 `target/real-app-matrix-final.json`，原始 stdout/stderr 和 worker trace 位于 `target/real-app-matrix/`。原始 Probe 包含本机环境变量，未复制进本跟踪文档或提交物；需要复核时应在本机读取并按敏感信息处理。

已执行并通过：

- Windows PowerShell AST parse：`PARSE_OK`。
- `Save-Json` 小 fixture：两次写入同一路径（覆盖分支）后 round-trip `SAVE_JSON_SELF_TEST_OK`，临时文件和 self-test 文件均清理。
- 正常外层 WMI worker 运行：harness process exit `0`，`harness_status=completed`。
- wrapper owned-handle `GetExitCodeProcess`：CreateProcess/AsUser、cmd、PowerShell 均真实 exit code `0`；Host/Aura Chrome 均真实 exit code `13`。
- `Stop-OwnedTree` 稳定句柄 safety fixture：错误 stamp 拒绝且进程仍存活，正确 stamp 同句柄终止并确认退出，状态 `Verified`。
- 当前运行根目录 containment/reparse/残留检查：通过。
- 运行后 owned Chrome 临时 profile 检查：`0` 个现存命令行命中。

本轮只新增真实入口矩阵工具和证据文档，没有运行 `cargo fmt`，没有修改 Runtime、Supervisor、Core、CLI 或 App 源码，也没有提交 Git。P4–P8 需要 VM、驱动、Verifier、恢复和长期运行的证据仍未由这份本机验收覆盖。
