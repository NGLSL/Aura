Parent: .scratch/envbox-v02/spec.md

# 36: WindowsApps 启动策略 — Package Identity + Trust Level

**What to build:** 按打包模型（而非目录）识别 WindowsApps 目标；选择器展示能力徽章；AppContainer 不支持注入。

**Blocked by:** （策略先落地；完整 AUMID 激活后端可后置）

**Status:** open

- [x] 分类模型：Win32 / Packaged Win32 / AppContainer / PackagedUnknown
- [x] Capability Probe（路径 + AppxManifest 启发式）→ 可注入 / 延迟注入 / 不支持注入
- [x] 添加应用选择器展示来源、包类型、注入能力徽章
- [x] AppContainer / UWP：标记不支持 Runtime Injection，禁止静默无虚拟化运行
- [ ] `LaunchTarget::PackagedApp { aumid, package_family_name }` + `IApplicationActivationManager`
- [ ] 进程级 Capability Probe（TokenIsAppContainer / Integrity / ProcessSignaturePolicy）
- [ ] PSF / Package Debug Settings（研究向，不进 V0.x 默认路径）

## 决策

WindowsApps **不要按目录处理**，按 **Package Identity + Trust Level + Runtime Behavior** 分：

| 类型 | 启动 | 注入 | V0.2 |
|------|------|------|------|
| Packaged Win32 / Full Trust / Medium IL | ActivateApplication(AUMID) 或 exe | 激活后注入（有 race window） | 标「延迟注入」，后端后置 |
| Packaged Win32 + AppContainer | AUMID | 受 Package/Token 限制 | **不支持** |
| UWP / AppContainer | AUMID | 同上 + 可能 MicrosoftSignedOnly | **不支持** |

不采用：

- **PSF 重打包**：破坏原商店签名/更新/许可，不作默认方案。
- **IPackageDebugSettings 长期运行**：改变 package lifecycle，仅作后续研究。

普通 Win32 保持：`CreateProcess(SUSPENDED) → Inject → Resume`。

## 实现（本轮）

- `crates/envbox-app/src/package.rs`：`Packaging` / `InjectionSupport` / `Capability` + `classify_target`
  - `shell:AppsFolder\…!App`、裸 AUMID → AppContainer / 不支持
  - `WindowsApps` 路径 → 找 `AppxManifest.xml`：`FullTrustApplication` / `Executable=` → Packaged Win32 / 延迟注入；WinRT EntryPoint → UWP / 不支持
  - 其余 → Win32 / 可注入
- `discover.rs`：分类字段、同 exe 只保留一条（start-menu 优先）
- 选择器：真图标 + 来源/包类型/注入能力徽章；选中带入后按能力给出状态提示

## Comments

### 2026-09-26 design

用户提供的策略：Full Trust 可激活后注入；AppContainer/签名策略 V0.2 直接拒绝；PSF 仅自控包可选。本轮先把「识别 + 徽章 + 不静默」落在添加应用交互，不改注入链路。
