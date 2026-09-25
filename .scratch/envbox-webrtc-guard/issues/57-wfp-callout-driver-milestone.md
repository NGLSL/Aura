Parent: .scratch/envbox-webrtc-guard/spec.md

# 57: WFP Callout Driver 里程碑（Phase 3+，单独评估）

**What to build:** （可选）小型内核 callout：`ALE_AUTH_CONNECT` 用 `FWPS_METADATA_FIELD_PROCESS_ID` 查 Session 成员，实现真正进程树级 UDP 策略。

**Blocked by:** 56

**Status:** resolved

> 受 AGENTS.md「不提前做 WFP Driver」约束：**不进阶段 1/2 交付**；仅当产品确认「只限本进程树、完全不影响同程序其他实例」且 56 无法满足时启动。

**评估结论（本 feature 交付时记录）：**

- 优先路径是 **Runtime 进程树内 WinSock UDP 约束**（注入进程才受 Hook，天然 Process-scoped，无驱动、Host 透明）。
- 用户态 WFP（`ALE_APP_ID`）**不作为产品承诺**：分不清 Aura Chrome 与用户 Chrome。
- 真正需要驱动的场景：非注入 helper / 内核旁路 UDP，且产品要求「绝不影响同程序 Session 外实例」时再启动本里程碑。
- 当前 `WebRtcPolicy::Strict` 在 Network Guard（Runtime UDP deny）未就绪时 **Startup Fail**，不静默降级。

- [x] 评估结论：是否需要驱动 vs 用户态 + 其他会话绑定方案 → **先 Runtime WinSock；驱动暂缓**
- [ ] 若做：callout 与 Session Registry PID 集对接；Fail 语义与安装/卸载生命周期明确
- [ ] 与 Aura Session stop / 崩溃残留的清理策略
- [ ] 不引入全局封禁；Host 其他流量零影响
- [ ] 签名/测试环境/发布成本单独里程碑，不与 Privacy Profile 绑定发版

## Comments
