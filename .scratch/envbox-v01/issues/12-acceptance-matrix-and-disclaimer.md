Parent: .scratch/envbox-v01/spec.md

# 12: 验收矩阵 + 安全声明收尾

**What to build:** V0.1 Done Definition 可勾选：Application/Profile/Runtime/Children/Host/Files/Probe 全部达标，并对外写清 EnvBox 不是安全边界。

**Blocked by:** 06 子进程传播, 07 Locale/Language/时区补全, 08 DNS View, 09 Registry Virtual View

**Status:** ready-for-agent

- [ ] 测试矩阵：`envbox-probe`、`cmd`、`powershell`、`git`、`node`、`python`、`notepad` + 一个真实 Node CLI Agent 场景
- [ ] Host vs US Probe 快照：虚拟化字段有明确差异；非虚拟化字段一致
- [ ] Host 的 Timezone/Language/Region/DNS 在跑完矩阵后仍为原值
- [ ] 目标进程仍可访问 `C:\`、`D:\`、Git repo、SSH keys、项目文件
- [ ] About/README 含非安全边界声明（与规格 §40 文案一致）
- [ ] 性能抽查：额外启动延迟理想 &lt;100ms / 可接受 &lt;300ms；Runtime 无 polling
