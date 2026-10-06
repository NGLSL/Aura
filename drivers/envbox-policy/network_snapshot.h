#ifndef ENVBOX_NETWORK_SNAPSHOT_H
#define ENVBOX_NETWORK_SNAPSHOT_H
#include "policy.h"
/* Resident, fixed-size values. PID is a WFP endpoint lookup index only; the
 * adapter publishes it from the retained process object, with its identity.
 * Every access requires the adapter's snapshot spin lock. No object is
 * dereferenced and no allocation or Windows lookup occurs in classification. */
typedef struct eb_network_entry {
    uint64_t endpoint_pid;
    eb_identity identity;
    uint8_t occupied, bound;
    uint32_t policy;
} eb_network_entry;
typedef struct eb_network_snapshot { eb_network_entry entries[EB_POLICY_CAPACITY]; } eb_network_snapshot;
/* Unknown/Host return EB_HOST (translate to CONTINUE, never hard PERMIT).
 * A known member using an unsupported IPv4 protocol is denied. */
eb_decision eb_network_classify(const eb_network_snapshot *, uint64_t endpoint_pid,
                                uint32_t protocol);
typedef enum eb_network_action { EB_ACTION_PRESERVE, EB_ACTION_CONTINUE,
                                 EB_ACTION_BLOCK } eb_network_action;
/* WFP permits a veto of an existing PERMIT even without ACTION_WRITE. */
eb_network_action eb_network_resolve_action(eb_decision decision, int action_write,
                                           int existing_permit);
#endif
