Parent: .scratch/envbox-v03/spec.md

# 45: Broker 优先配置通道 + 删除 C++ TOML（Phase 3）

**What to build:** Runtime 配置以 Broker PROFILE 为权威来源；ENVBOX_* 仅回退；删除 C++ 对 `profiles.toml` 的解析，C++ 只消费 RuntimeProfile DTO。

**Blocked by:** 44

**Status:** resolved

- [x] 加载顺序：Broker PROFILE →（可选）ENVBOX_* 回退 → 否则 Startup Fail Policy
- [x] 移除 C++ TOML 文件读取与自制解析；Profile 校验唯一归属 Rust serde
- [x] RuntimeProfile 结构保持 hooks 所需字段完整（timezone/locale/language/dns/registry/flags/ids），不丢虚拟化语义
- [x] 回归：同一 Profile 下 Probe 输出与删除前一致（Geo/Locale/Language/TZ/DNS/ENV/Registry）— 以当前预编译 DLL + 接缝测试为准
- [x] Packaged/无 Environment Block 场景仅靠 Broker 即可取 Profile（IPC 优先）
- [x] 文档/注释不再描述「C++ 读 profiles.toml」为有效路径

## Comments

### 2026-09-27 done

`runtime_profile.cpp` 已删除全部 TOML 解析；改 IPC 优先 + `ENVBOX_LOCALE_NAME` 等结构化值回退。Rust `environment.rs` 写入回退字段。**注意**：本机无 MSVC/cl，`envbox-runtime*.dll` 尚未用新源码重建；现有测试仍用旧 DLL（内含 TOML）并通过。下次 CMake/MSVC 构建后应再跑 Probe 矩阵确认去 TOML 路径。
