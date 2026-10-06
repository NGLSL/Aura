# 可信服务、启动前绑定与 IPv4 WFP 增量

Date: 2026-10-06
Baseline: `6f40a80`
Branch: `dev`
Scope: 用户明确要求实现下一批可信控制服务、启动前绑定和 IPv4 WFP 原型。只做源码、构建和无需加载驱动的验证；不安装/启动服务、不加载驱动、不修改宿主网络或信任，IPv6 继续延期。

## 实施契约

- 服务名称固定 AuraPolicyService，primary token 要求 session-zero LocalSystem 与 enabled dedicated service SID。设备/IOCTL 仍仅接受该身份，不凭 JSON PID、句柄或 SID 自认领。
- 服务管理请求来自本地 pipe；服务器实际 peer process handle/token 与 pipe impersonation 身份独立核对，普通用户目标以其 primary token 运行。文件权限预检可以短暂 impersonate，但必须在 Create/driver IOCTL 前 RevertToSelf。
- 复用 Launcher Session、Detours 创建时注入、Job creation attribute、Runtime identity 和 entry gate。独立服务入口接收明确 token/安装目录 Runtime bundle，并在第一次 primary Resume 前向驱动绑定实际保留 handle；不复制一套启动业务，不继承 LocalSystem 环境或可变 ENV DLL 路径。
- 服务 Runtime Broker 明确限制到实际用户 token 的 SID/integrity/session；用户 pipe 权限不包含 FILE_CREATE_PIPE_INSTANCE。Runtime 使用具体读写数据权限，并验证实际 server 的服务身份，trusted bootstrap 在初始化锁存且不得在 IPC 失败后回退 ENV Profile。
- 网络分类使用 callback-held process object 建立的 endpoint PID 索引，退出先撤掉 snapshot，再释放对象。分类读取 resident scalar snapshot，使用 spin lock；不在分类中使用 PASSIVE pushlock、分配、PID lookup 或 pageable API。
- 当前 production Container/Strong 门槛保持关闭。源码链路不等于可信服务已部署或 WFP 实际包过滤；cross-bitness helper、克隆、BFE、旧连接撤销、卸载/故障恢复等资格单独记录。

## 关键平台依据

CreateProcessAsUser 不自动构建用户环境，使用已认证 primary token 的 CreateEnvironmentBlock；服务实际创建者与目标 token 是不同事实。[Microsoft CreateProcessAsUser](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw)。

Named pipe 的 FILE_GENERIC_WRITE 包含 FILE_CREATE_PIPE_INSTANCE 权限，所以服务 broker/client 使用具体 data 权限，并 independently 验证 server 身份。[Microsoft pipe access](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)。

WFP 分类没有普通 ACTION_WRITE 时仍可否决已有 hard PERMIT；不能提前跳过自身 Deny。Host 分支不得覆写无权限的原结果。[Microsoft classify output](https://learn.microsoft.com/en-us/windows/win32/api/fwpstypes/ns-fwpstypes-fwps_classify_out0)。

## 运行证据

### 服务与 Launcher

新增 `crates/envbox-policy-service/`，包含 SCM host、实际 token/peer 认证、bounded protocol、受保护安装 bundle lease 和 PrototypeEngine。引擎实际调用服务 Launcher 接缝，在 before_resume 中对当前进程持有的真实 target handle 发送 IOCTL；持有 Session/Job/Broker，Status/Stop 核对 owner 与实际 Job。Container UUID 与 Profile UUID 分离，策略摘要由服务读取的权威快照和启动 envelope 计算，请求成功/失败均可按 exact owner/payload 重放。

生产 SCM dispatcher 的 Launch/Status/Stop 仍明确返回 unqualified；没有用户可开启的 ENV/命令行开关。prototype 引擎没有接入生产 GUI/CLI/Supervisor，没有持久 service journal 或重启恢复；不能宣称完整服务部署路径完成。当前批准管理镜像是安装目录里的固定 Aura/envbox 文件，核对实际 file ID/hash、标准 Runtime 模块与真实 owned Job；输入路径先固定本地非 reparse 祖先/文件，再解析，不让 System canonicalize 用户 UNC 或设备路径。

服务初版定向 tests 9/9、cargo check 通过，审查补强后为 14/14（见下）。Launcher 新接缝包含 token 派生用户环境、Detours 自定义 CreateProcessAsUser callback、显式安装 Runtime、实际 primary handle Job membership 和首次 Resume 前绑定；服务跨架构 helper 明确拒绝。实际 READ 允许但 EXECUTE 被拒的临时 ACL，以及附加服务端 pipe instance AccessDenied 的 OS 对照通过。

原生启动三负例实际通过：missing Runtime 没有调用绑定；实际 callback bind 失败不执行目标入口；普通用户伪服务 Broker 不能完成可信 Runtime 初始化、不执行入口。`target/service-launcher-native-binding.log`，fresh fixture PID 15188 / Runtime 0。该测试 callback 只模拟 bind 成败，不是假装真实内核绑定。

### Runtime 可信 bootstrap

生产 service bootstrap fixture 双架构 24/24、exit 0，fresh WMI 23348 / Runtime 0，`target/service-bootstrap-3e9ffe14c3fc4fd69cbac3bf5d7f80df/result.json`。每架构实际完整 ENV Profile 的 legacy LoadLibrary 成功；trusted fake service/invalid flag 均拒绝 1114、不回退 ENV。具体 client 权限可连接，但额外 server instance 被 OS 拒绝 error 5。flag missing/1/0/empty/long 与初始化后修改的锁存检查通过。真实 System/service-SID 正向和可信子进程传播仍未验。

最终冻结 pair `target/service-bootstrap-runtime/`：

```text
x64 1454D005485AF5B1C66158B5CF3489515A1BDAF35238C11F9DCB8E9C2C59516B
x86 6BB02A675E6C978E1E097D10DC13F70D537F92C6FC32A2733D85CC2220C4F3EE
```

初次 fixture raw 24 项结束后因 PowerShell 5 extended string 属性报告序列化卡住；仅终止已确认本 fixture controller，旧目录 `target/service-bootstrap-4a89dc59ade74f6890335902f81abac0/` 保留。修报告读取方式后独立 fresh run 退出 0，不把旧 run 算成功。

### IPv4 WFP

真实 ALE_AUTH_CONNECT_V4 callout、dynamic engine session、transaction sublayer/callout/filter 注册源已编译链接，包含 loopback，无 EXE path 排除。snapshot 在 PASSIVE 归属锁内构建，spin lock 发布/读取，退出先移除再释放引用。实际共享 Host/Pending/Deny/action decision host fixture x64/x86 各 177 assertions 与 C++ linkage 通过，fresh controller 11336 / Runtime 0，`target/envbox-policy-fixture-0cbdf5cb12b24b9cb66cafc129957652/result.json`。

SYS `/kernel /W4 /WX` 编译链接与 PE/import 检查通过，实际 imports 为 fwpkclnt.sys、ntoskrnl.exe，`target/envbox-policy-wfp-82e54532877b419ba39a1d55a23821b1/result.json`，SHA `C460B8D8385425B6CB0D1DA003CD77C9CBE9AB2CFE05DA29279A626EC8020AFD`。没有实际 WFP 网络包/内核 DDI 验证。

独立 risk review 发现 hard-PERMIT veto P1，已通过生产共享 action helper 修复并增量复核；无 ACTION_WRITE 的 Deny/Pending 仍可否决已有 PERMIT，unknown Host 与已有 BLOCK 保留原结果。回滚先移除 dynamic filter 再 unregister runtime；极端 unregister/rundown 失败保留 disabled resident image 并拒绝所有 IRP，避免 DriverEntry 失败释放仍有回调引用的 image。该例外 success 只固定镜像，不代表后端初始化成功；没有真实 DDI fault injection。

实际 metadata、endpoint 委派/继承、BFE stop/restart、Native/PSS clone、旧流 revoke 的 reauthorization/drain、卸载和正式创建窗口仍缺资格。原型不能启用 Container/Strong，也没有 host 安装/加载。

### 工作区集成

Rust 1.99 locked workspace build/test 全部 exit 0：443 passed / 0 failed / 35 ignored。使用上述 pair，fresh WMI 30200 / Runtime 0，`target/workspace-service-wfp-final-{build.log,test.log,result.json}`。ignored 不算通过；native ordering fixture 等指定负向另外实际执行。这次全量发生在最终 Spec 增量补强之前，补强后针对受影响模块再次验证，不能把先前全量当作最终新增代码已运行。

同一冻结 pair 的实际混合架构恢复 4/4：64→32、32→64 的 root live/exited、crash/restart 及 Stop A 保持 B/Host 场景通过，fresh WMI 24796 / Runtime 0，28.02 秒、exit 0。`target/mixed-service-wfp-final.log`、`.stderr.log`、`.exit`、`-host.log`。

### 最终审查补强

Spec 初审发现 Container/Profile/RunSnapshot 权威绑定、直接启动输入路径和 Runtime 裸路径租约三项缺口，均在提交前处理并增量复审。WDM 的 hard-PERMIT veto P1 也已修复，源码复审无新增可执行问题。Standards 与 Spec 的最终结果分别记录，不以构建成功代替规格验收。

Launcher 现在通过 opaque `TrustedRuntimeBundle` 传递固定双架构文件与稳定受保护租约，没有公开 unchecked 构造。仅卷根允许普通用户 ADD_SUBDIRECTORY，卷根以下祖先和安装目录仍拒绝全部创建、修改、删除、DACL/owner 权限，防止新增 sidecar DLL。实际只读 C 盘根目录 ACL 证实普通用户可创建子目录，原全部 write-bit 拒绝规则会误拒正常安装。实际 C 根和 Program Files 保护租约正向、普通临时目录 bundle 反向测试通过，没有修改宿主 ACL。

补强后的服务启动接缝定向 9 passed / 0 failed / 1 ignored；原生三负例 3/3，fresh controller 17420 / Runtime 0，`target/service-launcher-leased-native-binding.log`。这里的 Runtime 安装信任由仅 `cfg(test)` 的 crate-private fixture 构造模拟，实际 Detours、绑定回调、拒绝伪服务和不执行入口仍真实运行；不把它算作受保护正式安装的正向证明。

服务最终定向 14/14，locked check 通过。协议仅接收 snapshot/profile/configuration/digest 引用；服务通过实际认证用户 token 的 KnownFolder(LocalAppData) 派生根目录，在用户 impersonation 中只读 Container、Profile 和 RunSnapshot。各文档限制一 MiB，并固定非 reparse 路径；不使用 System 默认目录、ENVBOX_CONFIG_ROOT 或客户端传入配置根。ID、Container/Profile 绑定、configuration/content digest 与 snapshot.validate 均检查，使用存盘快照的 effective Profile；live Profile 编辑不改变该实例的快照。Driver 摘要纳入服务读取的快照和独立 Host/Deny 启动 envelope。

四项实盘 snapshot fixture 在当前用户 native impersonation 下覆盖缺失、篡改、错误绑定和 freeze；不是 System/service-SID 正向验证。Standards 初审的单实例 silent-peer P2 已修复：payload 读取前核对真实进程句柄、generation、token 和批准管理镜像，未批准者立即断开；读取后继续做 named-pipe impersonation 身份核对。实际无 payload 连接反向 0 ms 拒绝，小于测试 2 秒门槛，未等待旧 5 秒窗口。真实服务中批准客户端的正向、吞吐和停机仍未验。

最终工作区重验：fresh WMI 6604 / Runtime 0，Rust 1.99 locked build/test 均 exit 0，451 passed / 0 failed / 35 ignored，`target/workspace-service-wfp-reviewed-{build.log,test.log,result.json}`。这轮包括快照、opaque bundle 与预认证补强；随后仅统一服务路径 guard 到已有严格 helper，并扩展原有 consumer 测试。该最后补丁后 fresh WMI 19836 / Runtime 0 的服务定向再次 14/14，`target/service-policy-reviewed-{test.log,result.json}`。不把修补前运行宣称为最后 guard 的验证；最后 guard 的定向测试覆盖实际租约入口。

所有变更 Rust 文件 rustfmt check、三个 PowerShell 工具 AST、git diff check 通过；Launcher x86 check 通过。实际构建的 SYS、Runtime pair 与共享 C fixture 在最后 Rust 接口改动中未改动，不重复冒充新的内核运行证据。

## Standards

初审一项 P2：未批准 silent peer 可占用单实例 pipe 的请求读取窗口。提交前通过真实管理镜像预认证和读后完整身份核对修复；增量复审无剩余 actionable finding。Broker 认证未长持进程句柄的稳健性风险保留为后续资格项，审查未证实越权绕过，未列为当前 P1/P2。

## Spec

初审三项：authoritative Container/Profile/RunSnapshot 绑定 P1、直接入口输入路径 P2、裸 Runtime path 租约 P2。提交前均修复。增量复审又指出服务租约入口仍使用较宽 guard；改为复用唯一严格 validator，在任何文件打开前检查，并扩展 consumer 负例。最终增量复审无剩余 actionable finding。

这表示本轮增量源码缺陷已闭环，不表示总规格 P0–P8 已验收。生产 SCM dispatch、管理客户端、持久 service journal/restart recovery、真实服务正向、驱动/WFP/存储 backend、签名安装、Verifier 与真实应用矩阵仍各自保持未完成资格；IPv6 按用户要求后置，不阻塞当前 IPv4 源码交付。

Standards：0 剩余 findings；Spec：0 剩余本轮增量 findings；WDM：hard-PERMIT veto P1 已修复，0 新增源码 findings。运行资格缺口仍明确保留。
