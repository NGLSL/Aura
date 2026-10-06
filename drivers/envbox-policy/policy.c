#include "policy.h"

static void zero(void *p, size_t n) { size_t i; uint8_t *b=p; for(i=0;i<n;i++) b[i]=0; }
static void copy(void *d,const void *s,size_t n) { size_t i; uint8_t *a=d; const uint8_t *b=s; for(i=0;i<n;i++) a[i]=b[i]; }
static int equal(const void *a,const void *b,size_t n) { size_t i; const uint8_t *x=a,*y=b; for(i=0;i<n;i++) if(x[i]!=y[i]) return 0; return 1; }
static int nonzero(const uint8_t *p,size_t n) { size_t i; for(i=0;i<n;i++) if(p[i]) return 1; return 0; }
static uint32_t le32(const uint8_t *p) { return (uint32_t)p[0]|((uint32_t)p[1]<<8)|((uint32_t)p[2]<<16)|((uint32_t)p[3]<<24); }
static uint64_t le64(const uint8_t *p) { return (uint64_t)le32(p)|((uint64_t)le32(p+4)<<32); }
static int valid_id(eb_identity p) { return p.process_key!=0 && p.creation_time!=0; }
static eb_slot *find(eb_table *t,eb_identity p) { size_t i; for(i=0;i<EB_POLICY_CAPACITY;i++) if(t->slots[i].occupied && t->slots[i].identity.process_key==p.process_key && t->slots[i].identity.creation_time==p.creation_time) return &t->slots[i]; return 0; }
static eb_result channel(const eb_table *t,uint64_t generation) {
    if(!t || !t->initialized) return EB_UNAUTHORIZED;
    if(generation!=t->generation) return EB_STALE_GENERATION;
    if(!t->connected) return EB_DISCONNECTED;
    return EB_OK;
}
void eb_initialize(eb_table *t) { if(t) zero(t,sizeof(*t)); }
eb_result eb_connect(eb_table *t,const uint8_t owner[32],uint64_t generation,int verified) {
    if(!t || !owner || !verified || !generation || !nonzero(owner,32)) return EB_UNAUTHORIZED;
    if(t->initialized) {
        if(!equal(owner,t->owner,32)) return EB_UNAUTHORIZED;
        if(generation<=t->generation) return EB_STALE_GENERATION;
        /* Replacing a live controller is forbidden; explicitly disconnect it. */
        if(t->connected) return EB_CONFLICT;
    }
    copy(t->owner,owner,32); t->generation=generation; t->initialized=1; t->connected=1;
    return EB_OK;
}
eb_result eb_disconnect(eb_table *t,uint64_t generation) {
    eb_result r=channel(t,generation); if(r!=EB_OK) return r;
    t->connected=0; return EB_OK;
}
/* LE layout: version:u32, length:u32, op:u32, policy:u32, generation:u64,
 * process_handle:u64, owner:32, container:16, instance:16, digest:32.
 * reserved:8 (all zero). No extensible trailing fields. */
eb_result eb_decode(const void *wire,size_t length,eb_message *out) {
    const uint8_t *p=wire; eb_message m;
    if(!p || !out || length!=EB_POLICY_WIRE_SIZE) return EB_INVALID_WIRE;
    if(le32(p)!=EB_POLICY_VERSION || le32(p+4)!=EB_POLICY_WIRE_SIZE) return EB_INVALID_WIRE;
    if(nonzero(p+128,8)) return EB_INVALID_WIRE;
    zero(&m,sizeof(m)); m.operation=le32(p+8); m.policy=le32(p+12);
    m.generation=le64(p+16); m.process_handle=le64(p+24);
    copy(m.owner,p+32,32); copy(m.container,p+64,16); copy(m.instance,p+80,16); copy(m.digest,p+96,32);
    if((m.operation!=EB_POLICY_BIND && m.operation!=EB_POLICY_REVOKE) || m.policy>EB_POLICY_DENY || !m.generation || !m.process_handle || !nonzero(m.owner,32) || !nonzero(m.container,16) || !nonzero(m.instance,16) || !nonzero(m.digest,32)) return EB_INVALID_WIRE;
    if(m.operation==EB_POLICY_REVOKE && m.policy!=EB_POLICY_DENY) return EB_INVALID_WIRE;
    *out=m; return EB_OK;
}
eb_result eb_authorize(const eb_table *t,uint64_t generation,const eb_message *m) {
    eb_result r=channel(t,generation); if(r!=EB_OK) return r;
    if(!m || m->generation!=generation) return EB_STALE_GENERATION;
    if(!equal(m->owner,t->owner,32)) return EB_UNAUTHORIZED;
    /* Revalidate even when caller accidentally bypassed eb_decode. */
    if((m->operation!=EB_POLICY_BIND && m->operation!=EB_POLICY_REVOKE) || m->policy>EB_POLICY_DENY || !m->process_handle || !nonzero(m->container,16) || !nonzero(m->instance,16) || !nonzero(m->digest,32) || (m->operation==EB_POLICY_REVOKE && m->policy!=EB_POLICY_DENY)) return EB_INVALID_WIRE;
    return EB_OK;
}
eb_result eb_mark_pending(eb_table *t,uint64_t generation,eb_identity p) {
    size_t i; eb_result r=channel(t,generation); if(r!=EB_OK) return r;
    if(!valid_id(p)) return EB_UNAUTHORIZED;
    if(find(t,p)) return EB_OK;
    for(i=0;i<EB_POLICY_CAPACITY;i++) if(!t->slots[i].occupied) {
        zero(&t->slots[i],sizeof(t->slots[i])); t->slots[i].identity=p; t->slots[i].occupied=1; return EB_OK;
    }
    return EB_CAPACITY;
}
eb_result eb_apply(eb_table *t,uint64_t generation,const eb_message *m,eb_identity p) {
    eb_slot *s; eb_result r=eb_authorize(t,generation,m); if(r!=EB_OK) return r;
    if(!valid_id(p)) return EB_UNAUTHORIZED;
    s=find(t,p); if(!s) return EB_NOT_FOUND;
    if(s->bound) {
        if(!equal(s->container,m->container,16) || !equal(s->instance,m->instance,16) || !equal(s->digest,m->digest,32)) return EB_CONFLICT;
        if(m->operation==EB_POLICY_REVOKE) { s->policy=EB_POLICY_DENY; s->bound=2; return EB_OK; }
        if(s->bound==2 || s->policy!=m->policy) return EB_CONFLICT;
        return EB_OK;
    }
    if(m->operation==EB_POLICY_REVOKE) return EB_NOT_FOUND;
    copy(s->container,m->container,16); copy(s->instance,m->instance,16); copy(s->digest,m->digest,32);
    s->policy=m->policy; s->bound=1; return EB_OK;
}
eb_result eb_process_exit(eb_table *t,eb_identity p) {
    eb_slot *s; if(!t || !valid_id(p)) return EB_UNAUTHORIZED;
    s=find(t,p); if(!s) return EB_NOT_FOUND;
    zero(s,sizeof(*s)); return EB_OK;
}
eb_decision eb_classify(const eb_table *t,eb_identity p,uint32_t family,uint32_t direction,uint32_t protocol) {
    size_t i;
    if(!t || !valid_id(p) || family!=4 || direction!=1 || (protocol!=6 && protocol!=17)) return EB_DECISION_UNSUPPORTED;
    for(i=0;i<EB_POLICY_CAPACITY;i++) {
        const eb_slot *s=&t->slots[i];
        if(s->occupied && s->identity.process_key==p.process_key && s->identity.creation_time==p.creation_time) {
            if(s->bound!=1) return EB_PENDING_DENY;
            return s->policy==EB_POLICY_DENY ? EB_DENY : EB_HOST;
        }
    }
    return EB_HOST;
}
