Parent: .scratch/envbox-v02/spec.md

# 31: 边界 — x86 Runtime

**What to build:** `envbox-runtime32.dll` 注入链路，x86 目标与 x64 一致虚拟化。

**Blocked by:** 30（非硬性）

**Status:** resolved

- [x] 构建 `envbox-runtime32.dll`
- [x] x86 子进程注入 + Profile 继承
- [x] Probe/矩阵增加 x86 用例
- [x] 架构不匹配不静默降级

## Comments

### 2026-09-25 implementation (boundary 30–35)

- **构建**
  - Detours x86：`vcvars32 + nmake`（`D:\Tools\Detours\src`）→ `lib.X86/detours.lib`
  - Runtime32：`cmake -S runtime -B target/runtime-build32 -G "Visual Studio 17 2022" -A Win32 -DDETOURS_ROOT=D:/Tools/Detours && cmake --build ... --config Release`
  - 产物：`target/debug/envbox-runtime32.dll`（PE Machine `0x014c`）
- **解析**（`injection.rs`）
  - `pe_arch()` 读 PE `IMAGE_FILE_HEADER.Machine`
  - `resolve_runtime_dll_for_target()` 按目标 PE 位数只搜 `envbox-runtime32/64.dll`，**不跨位数回退**
  - 显式 `ENVBOX_RUNTIME_DLL` 与目标架构不符 → `InjectError::ArchitectureMismatch`（`refusing silent fallback`）
  - `ERROR_BAD_EXE_FORMAT(193)` → `ArchitectureLoad`
- **Evidence**
  - 单测：`pe_arch_reads_i386_and_amd64`、`explicit_runtime_dll_arch_mismatch_is_hard_error_and_match_ok`
  - 集成：`t31_runtime_dlls_have_expected_pe_machine`、`t31_arch_mismatch_refuses_silent_fallback`（runtime32 打 x64 probe 必须失败）、`t31_x86_inject_matrix_or_dll_product`（无 x86 probe 时验证 DLL PE）
  - 本机无 x86 测试 exe：完整 x86 注入矩阵 skip；PE 产物 + 不匹配不降级已覆盖
