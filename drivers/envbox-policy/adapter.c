#include <ntifs.h>
#include <wdmsec.h>
#include "policy.h"
#include "adapter.h"
#include "service_identity.h"

#define EB_TAG 'pAbE'
typedef struct eb_session {
    PEPROCESS controller;
    LONGLONG creation_time;
    uint64_t generation;
    BOOLEAN cleaned;
} eb_session;
typedef struct eb_device_state {
    EX_PUSH_LOCK lock;
    eb_table table;
    PFILE_OBJECT active_file;
    uint64_t next_generation;
    /* Every occupied slot owns exactly one callback-established reference. */
    PEPROCESS process_refs[EB_POLICY_CAPACITY];
    /* Keep disconnected controllers identifiable until OS exit, so they
     * cannot silently create Host processes after losing their channel. */
    PEPROCESS controller_refs[EB_POLICY_CAPACITY];
} eb_device_state;
static PDEVICE_OBJECT eb_device;
static const GUID eb_class_guid = {0x2572be60,0xa87e,0x4dd5,{0xa5,0x38,0xba,0x3e,0x75,0x18,0x94,0x31}};
static void lock_state(eb_device_state *s) { KeEnterCriticalRegion(); ExAcquirePushLockExclusive(&s->lock); }
static void unlock_state(eb_device_state *s) { ExReleasePushLockExclusive(&s->lock); KeLeaveCriticalRegion(); }
static NTSTATUS complete(PIRP irp,NTSTATUS status,ULONG_PTR information) {
    irp->IoStatus.Status=status; irp->IoStatus.Information=information;
    IoCompleteRequest(irp,IO_NO_INCREMENT); return status;
}
static BOOLEAN no_impersonation(void) {
    BOOLEAN copy,effective; SECURITY_IMPERSONATION_LEVEL level;
    PACCESS_TOKEN token=PsReferenceImpersonationToken(PsGetCurrentThread(),&copy,&effective,&level);
    if(token) { PsDereferenceImpersonationToken(token); return FALSE; }
    return TRUE;
}
/* All paged token queries occur at PASSIVE_LEVEL and outside state lock. */
static BOOLEAN controller_token(PEPROCESS process) {
    PACCESS_TOKEN token; PTOKEN_USER user=NULL; PTOKEN_GROUPS groups=NULL;
    BOOLEAN accepted=FALSE; ULONG i;
    token=PsReferencePrimaryToken(process);
    if(!token) return FALSE;
    if(!NT_SUCCESS(SeQueryInformationToken(token,TokenUser,(PVOID*)&user))) goto done;
    if(!RtlEqualSid(user->User.Sid,SeExports->SeLocalSystemSid)) goto done;
    if(!NT_SUCCESS(SeQueryInformationToken(token,TokenGroups,(PVOID*)&groups))) goto done;
    for(i=0;i<groups->GroupCount;i++) {
        ULONG attributes=groups->Groups[i].Attributes;
        if((attributes&EB_GROUP_ENABLED) && !(attributes&EB_GROUP_DENY_ONLY) &&
           RtlEqualSid(groups->Groups[i].Sid,(PSID)eb_service_sid)) { accepted=TRUE; break; }
    }
done:
    if(groups) ExFreePool(groups);
    if(user) ExFreePool(user);
    PsDereferencePrimaryToken(token);
    return accepted;
}
static BOOLEAN valid_requestor(PIRP irp) {
    return KeGetCurrentIrql()==PASSIVE_LEVEL && irp->RequestorMode==UserMode &&
           IoGetRequestorProcess(irp)==PsGetCurrentProcess() && no_impersonation() &&
           controller_token(PsGetCurrentProcess());
}
static NTSTATUS reject(PDEVICE_OBJECT device,PIRP irp) {
    UNREFERENCED_PARAMETER(device); return complete(irp,STATUS_INVALID_DEVICE_REQUEST,0);
}
static NTSTATUS create(PDEVICE_OBJECT device,PIRP irp) {
    PIO_STACK_LOCATION stack=IoGetCurrentIrpStackLocation(irp);
    eb_device_state *state=device->DeviceExtension; eb_session *session; eb_result result; ULONG i,available=EB_POLICY_CAPACITY;
    if(stack->FileObject->FileName.Length!=0 || !valid_requestor(irp)) return complete(irp,STATUS_ACCESS_DENIED,0);
    session=ExAllocatePool2(POOL_FLAG_NON_PAGED,sizeof(*session),EB_TAG);
    if(!session) return complete(irp,STATUS_INSUFFICIENT_RESOURCES,0);
    session->controller=PsGetCurrentProcess(); ObReferenceObject(session->controller);
    session->creation_time=PsGetProcessCreateTimeQuadPart(session->controller);
    if(session->creation_time<=0) {
        ObDereferenceObject(session->controller); ExFreePoolWithTag(session,EB_TAG);
        return complete(irp,STATUS_INVALID_DEVICE_STATE,0);
    }
    lock_state(state);
    if(state->active_file || state->next_generation==UINT64_MAX) {
        unlock_state(state); ObDereferenceObject(session->controller); ExFreePoolWithTag(session,EB_TAG);
        return complete(irp,STATUS_DEVICE_BUSY,0);
    }
    for(i=0;i<EB_POLICY_CAPACITY;i++) {
        if(state->controller_refs[i]==session->controller) { available=i; break; }
        if(!state->controller_refs[i] && available==EB_POLICY_CAPACITY) available=i;
    }
    if(available==EB_POLICY_CAPACITY) {
        unlock_state(state); ObDereferenceObject(session->controller); ExFreePoolWithTag(session,EB_TAG);
        return complete(irp,STATUS_INSUFFICIENT_RESOURCES,0);
    }
    session->generation=++state->next_generation;
    result=eb_connect(&state->table,eb_service_sid,session->generation,1);
    if(result!=EB_OK) {
        unlock_state(state); ObDereferenceObject(session->controller); ExFreePoolWithTag(session,EB_TAG);
        return complete(irp,STATUS_ACCESS_DENIED,0);
    }
    stack->FileObject->FsContext=session; state->active_file=stack->FileObject;
    if(!state->controller_refs[available]) { ObReferenceObject(session->controller); state->controller_refs[available]=session->controller; }
    unlock_state(state); return complete(irp,STATUS_SUCCESS,0);
}
static NTSTATUS cleanup(PDEVICE_OBJECT device,PIRP irp) {
    eb_device_state *state=device->DeviceExtension;
    PFILE_OBJECT file=IoGetCurrentIrpStackLocation(irp)->FileObject; eb_session *session;
    if(KeGetCurrentIrql()!=PASSIVE_LEVEL) return complete(irp,STATUS_INVALID_DEVICE_STATE,0);
    lock_state(state); session=file->FsContext;
    if(session && !session->cleaned) {
        session->cleaned=TRUE;
        if(state->active_file==file) { (void)eb_disconnect(&state->table,session->generation); state->active_file=NULL; }
    }
    unlock_state(state); return complete(irp,STATUS_SUCCESS,0);
}
static NTSTATUS close_session(PDEVICE_OBJECT device,PIRP irp) {
    eb_device_state *state=device->DeviceExtension;
    PFILE_OBJECT file=IoGetCurrentIrpStackLocation(irp)->FileObject; eb_session *session;
    if(KeGetCurrentIrql()!=PASSIVE_LEVEL) return complete(irp,STATUS_INVALID_DEVICE_STATE,0);
    lock_state(state); session=file->FsContext; file->FsContext=NULL;
    if(session && state->active_file==file) { (void)eb_disconnect(&state->table,session->generation); state->active_file=NULL; }
    unlock_state(state);
    if(session) { ObDereferenceObject(session->controller); ExFreePoolWithTag(session,EB_TAG); }
    return complete(irp,STATUS_SUCCESS,0);
}
static BOOLEAN active_session(eb_device_state *state,PFILE_OBJECT file,eb_session **out) {
    eb_session *session=file->FsContext;
    if(!session || session->cleaned || state->active_file!=file || !state->table.connected ||
       session->controller!=PsGetCurrentProcess() ||
       session->creation_time!=PsGetProcessCreateTimeQuadPart(PsGetCurrentProcess()) ||
       session->generation!=state->table.generation) return FALSE;
    *out=session; return TRUE;
}
static NTSTATUS control(PDEVICE_OBJECT device,PIRP irp) {
    PIO_STACK_LOCATION stack=IoGetCurrentIrpStackLocation(irp);
    eb_device_state *state=device->DeviceExtension; eb_session *session;
    ULONG code=stack->Parameters.DeviceIoControl.IoControlCode;
    ULONG input=stack->Parameters.DeviceIoControl.InputBufferLength;
    ULONG output=stack->Parameters.DeviceIoControl.OutputBufferLength;
    eb_message message; PEPROCESS target=NULL; NTSTATUS status; uint64_t generation; ULONG i; eb_identity identity; BOOLEAN registered=FALSE;
    if(!valid_requestor(irp)) return complete(irp,STATUS_ACCESS_DENIED,0);
    if(code!=IOCTL_EB_POLICY_SESSION && code!=IOCTL_EB_POLICY_APPLY) return complete(irp,STATUS_INVALID_DEVICE_REQUEST,0);
    if(code==IOCTL_EB_POLICY_SESSION) {
        if(input!=0 || output!=EB_SESSION_REPLY_SIZE || !irp->AssociatedIrp.SystemBuffer) return complete(irp,STATUS_INFO_LENGTH_MISMATCH,0);
        lock_state(state);
        if(!active_session(state,stack->FileObject,&session)) { unlock_state(state); return complete(irp,STATUS_ACCESS_DENIED,0); }
        generation=session->generation;
        for(i=0;i<8;i++) ((UCHAR*)irp->AssociatedIrp.SystemBuffer)[i]=(UCHAR)(generation>>(8*i));
        unlock_state(state); return complete(irp,STATUS_SUCCESS,EB_SESSION_REPLY_SIZE);
    }
    if(input!=EB_POLICY_WIRE_SIZE || output!=0 || eb_decode(irp->AssociatedIrp.SystemBuffer,input,&message)!=EB_OK) return complete(irp,STATUS_INVALID_PARAMETER,0);
    lock_state(state);
    if(!active_session(state,stack->FileObject,&session) || eb_authorize(&state->table,session->generation,&message)!=EB_OK) {
        unlock_state(state); return complete(irp,STATUS_ACCESS_DENIED,0);
    }
    generation=session->generation; unlock_state(state);
    /* x64 kernel handles set the top bit; reject them and negative pseudo
     * handles. WOW64 must provide an unsigned representable user handle. */
    if((message.process_handle>>63)!=0 || (IoIs32bitProcess(irp) && message.process_handle>UINT32_MAX)) return complete(irp,STATUS_INVALID_HANDLE,0);
    status=ObReferenceObjectByHandle((HANDLE)(ULONG_PTR)message.process_handle,
        EB_PROCESS_BIND_ACCESS,*PsProcessType,UserMode,(PVOID*)&target,NULL);
    if(!NT_SUCCESS(status)) return complete(irp,status,0);
    if(PsGetProcessCreateTimeQuadPart(target)<=0 || PsGetProcessExitStatus(target)!=STATUS_PENDING) status=STATUS_PROCESS_IS_TERMINATING;
    else {
        identity.process_key=(uint64_t)(ULONG_PTR)target;
        identity.creation_time=(uint64_t)PsGetProcessCreateTimeQuadPart(target);
        lock_state(state);
        if(!active_session(state,stack->FileObject,&session) || session->generation!=generation) status=STATUS_ACCESS_DENIED;
        else {
            for(i=0;i<EB_POLICY_CAPACITY;i++) {
                if(state->process_refs[i]==target && state->table.slots[i].occupied &&
                   state->table.slots[i].identity.creation_time==identity.creation_time) { registered=TRUE; break; }
            }
            /* Handles cannot adopt existing Host processes. Only actual
             * creating-controller callback registration establishes owner. */
            if(!registered) status=STATUS_NOT_SUPPORTED;
            else {
                eb_result result=eb_apply(&state->table,generation,&message,identity);
                status=result==EB_OK ? STATUS_SUCCESS : result==EB_CONFLICT ? STATUS_OBJECT_NAME_COLLISION : STATUS_ACCESS_DENIED;
            }
        }
        unlock_state(state);
    }
    ObDereferenceObject(target); return complete(irp,status,0);
}
static VOID process_notify(PEPROCESS process,HANDLE process_id,PPS_CREATE_NOTIFY_INFO info) {
    eb_device_state *state; eb_identity identity; eb_session *session; PEPROCESS release=NULL,controller_release=NULL; ULONG i;
    uint64_t generation=0; BOOLEAN candidate=FALSE,known_controller=FALSE;
    UNREFERENCED_PARAMETER(process_id);
    if(!eb_device) return;
    state=eb_device->DeviceExtension;
    identity.process_key=(uint64_t)(ULONG_PTR)process;
    identity.creation_time=(uint64_t)PsGetProcessCreateTimeQuadPart(process);
    lock_state(state);
    if(info) {
        /* Child inheritance has no qualified adapter/gate contract. A member
         * cannot silently create an unbound child, even after disconnect. */
        for(i=0;i<EB_POLICY_CAPACITY;i++) {
            if(state->process_refs[i]==PsGetCurrentProcess()) {
                info->CreationStatus=STATUS_NOT_SUPPORTED; unlock_state(state); return;
            }
            if(state->controller_refs[i]==PsGetCurrentProcess()) known_controller=TRUE;
        }
        if(state->active_file) {
            session=state->active_file->FsContext;
            if(session && !session->cleaned && session->controller==PsGetCurrentProcess() &&
               session->creation_time==PsGetProcessCreateTimeQuadPart(PsGetCurrentProcess())) {
                candidate=TRUE; generation=session->generation;
            }
        }
        if(known_controller && !candidate) { info->CreationStatus=STATUS_ACCESS_DENIED; unlock_state(state); return; }
        unlock_state(state);
        if(!candidate || !NT_SUCCESS(info->CreationStatus)) return;
        /* Actual creating thread context, never ParentProcessId. */
        if(info->IsSubsystemProcess || !info->FileObject || PsGetProcessCreateTimeQuadPart(process)<=0 ||
           !no_impersonation() || !controller_token(PsGetCurrentProcess())) {
            info->CreationStatus=STATUS_ACCESS_DENIED; return;
        }
        lock_state(state);
        session=state->active_file ? state->active_file->FsContext : NULL;
        if(!session || session->cleaned || session->controller!=PsGetCurrentProcess() ||
           session->generation!=generation || !state->table.connected) {
            info->CreationStatus=STATUS_ACCESS_DENIED; unlock_state(state); return;
        }
        for(i=0;i<EB_POLICY_CAPACITY;i++) if(!state->table.slots[i].occupied && !state->process_refs[i]) break;
        if(i==EB_POLICY_CAPACITY) { info->CreationStatus=STATUS_INSUFFICIENT_RESOURCES; unlock_state(state); return; }
        ObReferenceObject(process);
        if(eb_mark_pending(&state->table,generation,identity)!=EB_OK) {
            info->CreationStatus=STATUS_ACCESS_DENIED; unlock_state(state); ObDereferenceObject(process); return;
        }
        state->process_refs[i]=process;
        unlock_state(state); return;
    }
    if(state->active_file) {
        session=state->active_file->FsContext;
        if(session && session->controller==process && session->creation_time==(LONGLONG)identity.creation_time) {
            session->cleaned=TRUE; (void)eb_disconnect(&state->table,session->generation); state->active_file=NULL;
        }
    }
    for(i=0;i<EB_POLICY_CAPACITY;i++) {
        if(state->controller_refs[i]==process) { controller_release=state->controller_refs[i]; state->controller_refs[i]=NULL; }
        if(state->process_refs[i]==process && state->table.slots[i].occupied &&
           state->table.slots[i].identity.creation_time==identity.creation_time) {
            if(eb_process_exit(&state->table,identity)==EB_OK) { release=state->process_refs[i]; state->process_refs[i]=NULL; }
        }
    }
    unlock_state(state);
    if(release) ObDereferenceObject(release);
    if(controller_release) ObDereferenceObject(controller_release);
}
DRIVER_INITIALIZE DriverEntry;
NTSTATUS DriverEntry(PDRIVER_OBJECT driver,PUNICODE_STRING registry_path) {
    UNICODE_STRING name=RTL_CONSTANT_STRING(L"\\Device\\AuraPolicyPrototype");
    UNICODE_STRING link=RTL_CONSTANT_STRING(L"\\DosDevices\\AuraPolicyPrototype");
    UNICODE_STRING sddl=RTL_CONSTANT_STRING(EB_DEVICE_SDDL);
    eb_device_state *state; NTSTATUS status; ULONG i;
    UNREFERENCED_PARAMETER(registry_path);
    if(!RtlValidSid((PSID)eb_service_sid) || RtlLengthSid((PSID)eb_service_sid)!=32) return STATUS_INVALID_SID;
    for(i=0;i<=IRP_MJ_MAXIMUM_FUNCTION;i++) driver->MajorFunction[i]=reject;
    driver->MajorFunction[IRP_MJ_CREATE]=create;
    driver->MajorFunction[IRP_MJ_CLEANUP]=cleanup;
    driver->MajorFunction[IRP_MJ_CLOSE]=close_session;
    driver->MajorFunction[IRP_MJ_DEVICE_CONTROL]=control;
    /* Never provide a void unload handler pretending it can reject active
     * protection. This non-unloadable source prototype must not be loaded. */
    driver->DriverUnload=NULL;
    status=IoCreateDeviceSecure(driver,sizeof(eb_device_state),&name,FILE_DEVICE_UNKNOWN,
        FILE_DEVICE_SECURE_OPEN,FALSE,&sddl,&eb_class_guid,&eb_device);
    if(!NT_SUCCESS(status)) return status;
    state=eb_device->DeviceExtension; ExInitializePushLock(&state->lock); eb_initialize(&state->table);
    status=IoCreateSymbolicLink(&link,&name);
    if(!NT_SUCCESS(status)) { IoDeleteDevice(eb_device); eb_device=NULL; return status; }
    /* Register last: no fallible initialization remains after a callback can
     * run. Failed registration cannot leave a callback in an unloaded image. */
    status=PsSetCreateProcessNotifyRoutineEx(process_notify,FALSE);
    if(!NT_SUCCESS(status)) { IoDeleteSymbolicLink(&link); IoDeleteDevice(eb_device); eb_device=NULL; return status; }
    eb_device->Flags|=DO_BUFFERED_IO; eb_device->Flags&=~DO_DEVICE_INITIALIZING;
    return STATUS_SUCCESS;
}
