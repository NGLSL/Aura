# 05: 进程入口和真实应用覆盖矩阵

Stage: P0
Status: claimed
Blocked by: [04: 受支持子进程传播与身份确认](04-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** GUI/CLI 可依据实测矩阵准确说明支持入口和应用覆盖，避免将部分注入显示为完整隔离。

## 负责模块与契约

Probe、能力模型和展示接口。IsolationGuarantee 仅表示注入时机，能力状态使用 Verified/Partial/Unsupported/Unverified。

## 不包括

不实现新的代执行拦截，不禁用 Chromium sandbox，不安装或发布目标应用。

## 验收标准

- [ ] 矩阵分别记录 CreateProcess、AsUser、WithToken、Shell、Native、WMI、Packaged 与 console 入口的真实结果。
- [ ] 每项包含系统/权限、架构、Runtime 路径/身份和宿主对照，不从普通 Probe 推断其他入口。
- [ ] 实际可用 Chromium 测试中 renderer 正确标 Partial/Unsupported；缺实测记录 Unverified，不能用模拟替代。
- [ ] 必要能力缺失的目标拒绝所请求保证；已存在宿主单实例不被当作新运行。
- [ ] GUI/CLI 同一事实输出原因；未支持路径没有循环注入或全局宿主拦截。

## 验证证据

提交可复现入口夹具与真实应用矩阵；无法安全取得真实应用证据时明确未完成该验收。

## 关联验收

A07、A08、A09、A14、F05、F09、F10。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。

2026-10-06 增量：独立 fresh WMI 真实入口矩阵确认 CreateProcess/AsUser、CMD、PowerShell 的 Runtime/Profile 输出及稳定句柄读取的退出码 `0`。Chromium 的 Host 与 Aura 同参数隐藏 headless 对照均提前退出 `13`，未观察到 renderer，因此浏览器保留 Unverified/NotObserved，不推断 Unsupported 或完整覆盖。只停止本次拥有的目标：稳定 handle 上读取 generation、核对后以同一 handle 终止并确认退出；实际错误 generation 控制样本被拒绝且目标仍存活，正确 generation 停止成功。Shell/WMI 能力边界、WithToken/Native/Packaged 未验仍逐行记录。本矩阵固定使用旧 V3 pair，不冒称新 async DLL 发布验收。见 [真实入口证据](../evidence/real-app-independent-final.md)。
