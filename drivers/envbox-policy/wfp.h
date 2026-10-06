#ifndef ENVBOX_WFP_H
#define ENVBOX_WFP_H
#include "network_snapshot.h"
void eb_network_initialize(void);
/* PASSIVE_LEVEL adapter lock -> snapshot spin lock; never reverse. */
void eb_network_publish(const eb_network_snapshot *snapshot);
NTSTATUS eb_wfp_start(PDEVICE_OBJECT device);
/* Initialization rollback only: no successful-driver unload path exists. */
NTSTATUS eb_wfp_rollback(void);
BOOLEAN eb_wfp_rollback_complete(void);
#endif
