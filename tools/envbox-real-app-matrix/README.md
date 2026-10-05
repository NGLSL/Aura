# EnvBox 真实入口矩阵

这是票 05 的独立验收夹具。它只验证当前工作区已有的启动入口和真实安装的 Chromium/Edge，不修改 Runtime、Supervisor、Core、CLI 或 App 源码，也不安装目标应用。

## 运行

从仓库根目录运行：

```powershell
powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File .\tools\envbox-real-app-matrix\run.ps1
```

外层脚本用 `Win32_Process.Create` 启动一个隐藏的、清除 `ENVBOX_*` 继承变量的 PowerShell worker。worker 为每次运行建立唯一的 `%TEMP%\envbox-real-app-matrix-{GUID}` 根目录、独立 Aura 配置和独立 Chrome `--user-data-dir`，运行结束后只在路径已解析到 `%TEMP%`、根目录不是 reparse point 且没有目标进程占用时递归清理。结果交接文件为 `target/real-app-matrix-final.json`，原始 stdout/stderr 在 `target/real-app-matrix/`。

脚本不使用用户现有浏览器 profile，不添加 `--no-sandbox`，不枚举或激活 Packaged 应用，不终止未由本次启动得到的 PID。浏览器停止前保存目标 PID 的 Windows creation stamp；停止时只用一次 `OpenProcess` 得到的稳定句柄调用 `GetProcessTimes`、比较 generation 并调用同一句柄 `TerminateProcess`，stamp 缺失或不匹配时拒绝停止，也不按 PID/PPID 快照递归杀子进程。真实浏览器入口前还会用另一个临时 profile 运行相同参数的未注入 Host Chrome 对照，以区分系统 headless 启动失败和 Aura 入口行为。

每个 `Start-Process` wrapper 在启动后立即保存自己的 Process handle。完成状态通过这个 owned handle 调用 `GetExitCodeProcess` 读取；初始的 `259/STILL_ACTIVE`、最终的真实 exit code、Win32 error 和 timeout 会分别记录，不通过晚到的 PID 重新打开句柄。

结果 JSON 先写入同目录唯一临时文件，再用同卷原子替换发布；外层 WMI 轮询如果读到旧内容或暂时无法解析的内容，会继续轮询到 bounded deadline。可以只验证发布/读取路径而不启动矩阵：

```powershell
powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File .\tools\envbox-real-app-matrix\run.ps1 -SaveJsonSelfTest
```

## 状态含义

每个入口单独输出 `Verified`、`Partial`、`Unsupported` 或 `Unverified`。`harness_status=completed` 只表示夹具完成并完成清理；`matrix_status` 保持 `partial-validation`，不会把 `exit_code=0` 解释成整张矩阵通过。Chromium root 和 renderer 分开记录；renderer 永远不会因为 root 通过而变成 `Verified`。

本夹具使用 `target/container-independent-final-runtime-v3/` 中的冻结 Runtime pair。它是可复现的旧验收输入，不代表异步 Runtime 或后续构建的最终发布身份；JSON 会写出实际路径和 SHA-256。
