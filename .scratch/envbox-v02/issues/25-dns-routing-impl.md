Parent: .scratch/envbox-v02/spec.md

# 25: DNS routing — 实现 + Fail Open

**What to build:** 在 `hooks_dns.cpp` 实现 VirtualView 解析路由。

**Blocked by:** 24 DNS routing — 解析入口 Hook 设计

**Status:** open

- [ ] VirtualView：按 Profile `servers` 顺序解析
- [ ] 全部失败 → Fail Open 到原 API
- [ ] Host 模式：解析路径完全不拦截
- [ ] 仅 Process Tree Instance 生效
- [ ] 单测/集成不依赖公网唯一路径

## Comments
