#include "network_snapshot.h"
eb_network_action eb_network_resolve_action(eb_decision decision,int action_write,int existing_permit) {
    if(decision!=EB_HOST) return action_write || existing_permit ? EB_ACTION_BLOCK : EB_ACTION_PRESERVE;
    return action_write ? EB_ACTION_CONTINUE : EB_ACTION_PRESERVE;
}
eb_decision eb_network_classify(const eb_network_snapshot *snapshot,uint64_t pid,uint32_t protocol) {
    size_t i;
    if(!snapshot || !pid) return EB_HOST;
    for(i=0;i<EB_POLICY_CAPACITY;i++) {
        const eb_network_entry *e=&snapshot->entries[i];
        if(e->occupied && e->endpoint_pid==pid) {
            if(!e->identity.process_key || !e->identity.creation_time || e->bound!=1) return EB_PENDING_DENY;
            if(protocol!=6 && protocol!=17) return EB_DENY;
            return e->policy==EB_POLICY_HOST ? EB_HOST : EB_DENY;
        }
    }
    return EB_HOST;
}
