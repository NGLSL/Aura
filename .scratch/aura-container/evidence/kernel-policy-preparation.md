# 内核策略后端源码准备

Date: 2026-10-06
Scope: 22/23 号票的无 VM 源码准备；不加载驱动、不启用 Container/Strong、不把离线状态机作为真实 WFP 隔离。

## 当前缺口

现有 `drivers/envbox-empty/envbox_empty.c` 只证明构建工具链，没有设备、IOCTL、进程回调或 WFP 后端。每用户 Supervisor/Broker 的身份不能直接升级为内核授权根，PID、环境变量和 Runtime 自报也不能用于认领受保护目标。

## 控制面实施决定

正式后端采用独立系统控制服务及专用 service SID 作为驱动控制端身份；GUI 和每用户 Supervisor 不直接写策略设备。未来服务负责验证请求者及目标归属、冻结策略，并在目标 Resume 前绑定。驱动验证设备访问、实际 controller process/token 和 session generation；不是对管理员或任意 LocalSystem 进程开放任意绑定。安装和启动该服务是后续独立步骤，本轮不修改 SCM。

这是完整后端的实施选择，尚不是当前已部署能力。SCM 将 service SID 放入服务 token，使对象访问可以按具体服务而非仅 LocalSystem 账号约束；restricted SID 的访问语义需要在服务/设备实际 ACL 验收时验证。[Microsoft service SID 契约](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-service_sid_info)、[驱动安全模型](https://learn.microsoft.com/en-us/windows-hardware/drivers/driversecurity/windows-security-model)。

wire 只携带固定长度版本化值和待验证 process handle，不携带用户指针或内核对象地址。未来 adapter 必须在请求者上下文中使用 `ObReferenceObjectByHandle` 的 UserMode 访问检查和 `PsProcessType` 验证，再将被引用的 process object 与 creation generation 交给核心；引用必须成对释放。不能把消息里的 PID 当作已验证对象。[Microsoft handle/object 契约](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-obreferenceobjectbyhandle)。

实际创建/退出观察使用文档化进程回调，取消注册时需要等待在途回调；回调、引用、锁和卸载的资格必须在隔离环境真实验证。[Microsoft 进程回调契约](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/nf-ntddk-pssetcreateprocessnotifyroutineex)。

## 当前实现边界

已实现 `drivers/envbox-policy/` 的纯 C 协议/绑定核心、WDM adapter 和双架构 host harness。核心包含版本/长度/保留字段、可信 controller generation、已验证进程对象 identity、不可变绑定、准确退出、断连保留保护、容量限制和 IPv4 Host/Deny 分类；host harness 每架构 157 个断言通过，C++ 调用方也实际编译链接通过。

WDM adapter 使用专用 AuraPolicyService SID 的设备 ACL；Create/每个 IOCTL 重新检查 LocalSystem primary token、enabled service SID、requestor context，拒绝 impersonation/deny-only SID。目标 handle 在 UserMode 下按 PsProcessType 验证，只能绑定回调已登记的实际 process object 与 creation time。callback 使用实际 creating thread 的进程归属，拒绝已绑定成员的受通知子进程创建；断连保留绑定和 controller 身份引用，进程退出精确清理。

实际固定 WDK/SDK 输入的 x64 SYS 编译、链接、PE/import 检查通过，SHA-256 `1F5991DA603B9902D54E4FD6B26327B0C5F78274BADF245A47E08A2CC2FA344B`，证据 `target/envbox-policy-driver/result.json`。该候选没有加载、没有真实可信服务、没有 WFP callout、没有正常用户应用端到端验证。DriverUnload 为 NULL；不能将不可卸载的资格候选安装到当前宿主。离线 harness 和 SYS 构建不证明 token/引用/锁在内核运行中正确，也不证明网络包被过滤。

进程回调只覆盖实际收到通知的创建路径。Native clone/PSS VA clone 的通知与归属覆盖必须在隔离环境验证；不能把未知对象的 Host 分类当作这些创建路径已被保护。此资格门槛记录于 22 号票，未满足前不得加载或声明完整 Container。不能通过将全部未知宿主进程改为 Deny 来掩盖归属缺口。

IPv6 按用户决定延期。文件/Registry、Allowlist/可信 DNS TTL、服务部署、WFP callout、创建窗口和 VM/Verifier 仍为后续项；未支持动作显式拒绝，不靠临时 Host 放行来填补后端。
