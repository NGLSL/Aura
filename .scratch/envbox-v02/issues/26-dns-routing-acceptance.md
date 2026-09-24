Parent: .scratch/envbox-v02/spec.md

# 26: DNS routing — 对比验收

**What to build:** VirtualView vs Host 解析结果可区分；Host 解析行为不变。

**Blocked by:** 25 DNS routing — 实现 + Fail Open

**Status:** open

- [ ] Host 模式解析与 Host 一致
- [ ] VirtualView 使用可控 fixture/本地 resolver 或文档化 contrast
- [ ] resolver 不可达不永久挂起
- [ ] 矩阵跑完后 Host DNS 不变

## Comments
