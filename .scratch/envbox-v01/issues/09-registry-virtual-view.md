Parent: .scratch/envbox-v01/spec.md

# 09: Registry Virtual View 白名单

**What to build:** 目标进程读取国际化/时区相关白名单注册表时看到 Profile 虚拟值；白名单外的注册表访问全部 Pass Through。不是 Registry Sandbox。

**Blocked by:** 07 Locale / Language / 时区换算补全

**Status:** ready-for-agent

- [ ] Hook `RegOpenKeyExW` / `RegQueryValueExW` / `RegGetValueW` 仅白名单命中
- [ ] 白名单至少覆盖 `HKCU\Control Panel\International` 与 `HKLM\SYSTEM\CurrentControlSet\Control\TimeZoneInformation`，以及 Probe 证实的必要相关路径
- [ ] 白名单外请求与 Windows 行为一致
- [ ] Probe 能对比 Host/Profile 的相关注册表读值
- [ ] 不扩大为完整 Registry virtualization
