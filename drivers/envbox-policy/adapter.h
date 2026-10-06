#ifndef ENVBOX_POLICY_ADAPTER_H
#define ENVBOX_POLICY_ADAPTER_H
/* Include ntifs.h or Windows SDK headers before this header. SESSION reads a
 * non-secret LE64 generation; APPLY consumes exactly the v1 policy wire.
 * Neither command accepts METHOD_NEITHER pointers or FILE_ANY_ACCESS. */
#define IOCTL_EB_POLICY_SESSION CTL_CODE(FILE_DEVICE_UNKNOWN,0x801,METHOD_BUFFERED,FILE_READ_ACCESS)
#define IOCTL_EB_POLICY_APPLY CTL_CODE(FILE_DEVICE_UNKNOWN,0x802,METHOD_BUFFERED,FILE_WRITE_ACCESS)
#define EB_SESSION_REPLY_SIZE 8u
/* SDK winnt.h values are not exposed by this cached ntifs.h. Host fixture
 * asserts these namespaced constants against the real SDK definitions. */
#define EB_GROUP_ENABLED 0x00000004u
#define EB_GROUP_DENY_ONLY 0x00000010u
#define EB_PROCESS_BIND_ACCESS 0x00001800u
#endif
