#include <ntifs.h>
#include <initguid.h>
#define NDIS_SUPPORT_NDIS6 1
/* Cached WDK NDIS headers use anonymous unions and deliberate alignment. */
#pragma warning(push)
#pragma warning(disable:4201 4324)
#include <fwpsk.h>
#include <fwpmk.h>
#pragma warning(pop)
#include "wfp.h"

static KSPIN_LOCK eb_network_lock;
static eb_network_snapshot eb_snapshot;
static HANDLE eb_engine;
static UINT32 eb_callout_id;
static const GUID eb_callout_key={0x3e8a6352,0x2d2d,0x4760,{0x9e,0x96,0x8f,0x62,0x47,0x91,0x70,0x8a}};
static const GUID eb_sublayer_key={0xe511a2d4,0xa0d3,0x4e5e,{0xac,0x1e,0xd5,0x87,0x1f,0x8e,0x6a,0x9b}};
void eb_network_initialize(void) { KeInitializeSpinLock(&eb_network_lock); RtlZeroMemory(&eb_snapshot,sizeof(eb_snapshot)); }
void eb_network_publish(const eb_network_snapshot *snapshot) {
    KIRQL old; KeAcquireSpinLock(&eb_network_lock,&old);
    eb_snapshot=*snapshot; KeReleaseSpinLock(&eb_network_lock,old);
}
static void NTAPI classify(const FWPS_INCOMING_VALUES0 *values,const FWPS_INCOMING_METADATA_VALUES0 *metadata,
    void *layer_data,const FWPS_FILTER0 *filter,UINT64 flow_context,FWPS_CLASSIFY_OUT0 *out) {
    KIRQL old; eb_decision decision=EB_HOST; eb_network_action action; UINT32 protocol;
    UNREFERENCED_PARAMETER(layer_data); UNREFERENCED_PARAMETER(filter); UNREFERENCED_PARAMETER(flow_context);
    if(values->layerId==FWPS_LAYER_ALE_AUTH_CONNECT_V4 &&
       (metadata->currentMetadataValues&FWPS_METADATA_FIELD_PROCESS_ID)) {
        protocol=values->incomingValue[FWPS_FIELD_ALE_AUTH_CONNECT_V4_IP_PROTOCOL].value.uint8;
        KeAcquireSpinLock(&eb_network_lock,&old);
        decision=eb_network_classify(&eb_snapshot,metadata->processId,protocol);
        KeReleaseSpinLock(&eb_network_lock,old);
    }
    action=eb_network_resolve_action(decision,(out->rights&FWPS_RIGHT_ACTION_WRITE)!=0,
                                    out->actionType==FWP_ACTION_PERMIT);
    if(action==EB_ACTION_BLOCK) { out->actionType=FWP_ACTION_BLOCK; out->rights&=~FWPS_RIGHT_ACTION_WRITE; }
    else if(action==EB_ACTION_CONTINUE) out->actionType=FWP_ACTION_CONTINUE;
}
static NTSTATUS NTAPI notify(FWPS_CALLOUT_NOTIFY_TYPE type,const GUID *key,const FWPS_FILTER0 *filter) {
    UNREFERENCED_PARAMETER(type); UNREFERENCED_PARAMETER(key); UNREFERENCED_PARAMETER(filter); return STATUS_SUCCESS;
}
BOOLEAN eb_wfp_rollback_complete(void) { return !eb_engine && !eb_callout_id; }
NTSTATUS eb_wfp_rollback(void) {
    NTSTATUS status;
    /* Closing the dynamic session removes filters BEFORE runtime unregister. */
    if(eb_engine) {
        status=FwpmEngineClose0(eb_engine);
        if(!NT_SUCCESS(status)) return status;
        eb_engine=NULL;
    }
    if(eb_callout_id) {
        status=FwpsCalloutUnregisterById0(eb_callout_id);
        if(!NT_SUCCESS(status) && status!=STATUS_FWP_CALLOUT_NOT_FOUND) return status;
        eb_callout_id=0;
    }
    return STATUS_SUCCESS;
}
NTSTATUS eb_wfp_start(PDEVICE_OBJECT device) {
    FWPS_CALLOUT0 runtime={0}; FWPM_SESSION0 session={0}; FWPM_SUBLAYER0 sublayer={0};
    FWPM_CALLOUT0 management={0}; FWPM_FILTER0 filter={0}; NTSTATUS status; BOOLEAN transaction=FALSE;
    runtime.calloutKey=eb_callout_key; runtime.classifyFn=classify; runtime.notifyFn=notify;
    status=FwpsCalloutRegister0(device,&runtime,&eb_callout_id); if(!NT_SUCCESS(status)) return status;
    session.flags=FWPM_SESSION_FLAG_DYNAMIC; session.displayData.name=L"Aura policy prototype";
    status=FwpmEngineOpen0(NULL,RPC_C_AUTHN_WINNT,NULL,&session,&eb_engine); if(!NT_SUCCESS(status)) goto failure;
    status=FwpmTransactionBegin0(eb_engine,0); if(!NT_SUCCESS(status)) goto failure; transaction=TRUE;
    sublayer.subLayerKey=eb_sublayer_key; sublayer.displayData.name=L"Aura process policy prototype"; sublayer.weight=0x100;
    status=FwpmSubLayerAdd0(eb_engine,&sublayer,NULL); if(!NT_SUCCESS(status)) goto failure;
    management.calloutKey=eb_callout_key; management.applicableLayer=FWPM_LAYER_ALE_AUTH_CONNECT_V4;
    management.displayData.name=L"Aura IPv4 outbound authorization";
    status=FwpmCalloutAdd0(eb_engine,&management,NULL,NULL); if(!NT_SUCCESS(status)) goto failure;
    filter.displayData.name=L"Aura retained process policy"; filter.layerKey=FWPM_LAYER_ALE_AUTH_CONNECT_V4;
    filter.subLayerKey=eb_sublayer_key; filter.weight.type=FWP_EMPTY;
    filter.action.type=FWP_ACTION_CALLOUT_UNKNOWN; filter.action.calloutKey=eb_callout_key;
    /* No app-path or loopback exclusion. CONTINUE preserves other firewalls. */
    status=FwpmFilterAdd0(eb_engine,&filter,NULL,NULL); if(!NT_SUCCESS(status)) goto failure;
    status=FwpmTransactionCommit0(eb_engine); if(!NT_SUCCESS(status)) goto failure;
    return STATUS_SUCCESS;
failure:
    if(transaction) (void)FwpmTransactionAbort0(eb_engine);
    (void)eb_wfp_rollback(); return status;
}
