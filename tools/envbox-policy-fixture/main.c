#include "policy.h"
#include <stdio.h>
#include <string.h>
#include <windows.h>
#include <sddl.h>
#include "adapter.h"
#include "service_identity.h"
#include "network_snapshot.h"
typedef char sdk_group_enabled[(EB_GROUP_ENABLED==SE_GROUP_ENABLED)?1:-1];
typedef char sdk_group_deny_only[(EB_GROUP_DENY_ONLY==SE_GROUP_USE_FOR_DENY_ONLY)?1:-1];
typedef char sdk_process_access[(EB_PROCESS_BIND_ACCESS==(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SUSPEND_RESUME))?1:-1];
typedef char sdk_ioctl_method[((IOCTL_EB_POLICY_APPLY&3)==METHOD_BUFFERED)?1:-1];
typedef char sdk_ioctl_access[(((IOCTL_EB_POLICY_APPLY>>14)&3)==FILE_WRITE_ACCESS)?1:-1];
static unsigned checks;
#define CHECK(expr) do { checks++; if(!(expr)) { fprintf(stderr,"FAIL line %d: %s\n",__LINE__,#expr); return 1; } } while(0)
static void put32(uint8_t *p,uint32_t n) { unsigned i; for(i=0;i<4;i++) p[i]=(uint8_t)(n>>(8*i)); }
static void put64(uint8_t *p,uint64_t n) { unsigned i; for(i=0;i<8;i++) p[i]=(uint8_t)(n>>(8*i)); }
static void wire(uint8_t *p) {
    memset(p,0,EB_POLICY_WIRE_SIZE); put32(p,1); put32(p+4,EB_POLICY_WIRE_SIZE);
    put32(p+8,EB_POLICY_BIND); put32(p+12,EB_POLICY_DENY);
    put64(p+16,1); put64(p+24,0x200); memset(p+32,1,32);
    memset(p+64,2,16); memset(p+80,3,16); memset(p+96,4,32);
}
int main(void) {
    eb_table t, before; eb_message m, original;
    uint8_t p[EB_POLICY_WIRE_SIZE], owner[32], other[32];
    eb_identity a={100,1000}, b={101,1000}, reused={100,2000}, host={102,1000}; unsigned i;
    { eb_network_snapshot n={0}; eb_network_entry *e=&n.entries[0];
      CHECK(eb_network_classify(&n,99,6)==EB_HOST);
      e->occupied=1; e->endpoint_pid=99; e->identity=a;
      CHECK(eb_network_classify(&n,99,6)==EB_PENDING_DENY);
      e->bound=1; e->policy=EB_POLICY_DENY;
      CHECK(eb_network_classify(&n,99,6)==EB_DENY); CHECK(eb_network_classify(&n,99,17)==EB_DENY);
      e->policy=EB_POLICY_HOST;
      CHECK(eb_network_classify(&n,99,6)==EB_HOST); CHECK(eb_network_classify(&n,99,17)==EB_HOST);
      CHECK(eb_network_classify(&n,99,1)==EB_DENY); CHECK(eb_network_classify(&n,100,1)==EB_HOST);
      e->bound=2; CHECK(eb_network_classify(&n,99,6)==EB_PENDING_DENY);
      e->bound=1; e->identity.creation_time=0; CHECK(eb_network_classify(&n,99,6)==EB_PENDING_DENY);
      memset(e,0,sizeof(*e)); CHECK(eb_network_classify(&n,99,6)==EB_HOST);
      e->occupied=1; e->endpoint_pid=99; e->identity=reused; e->bound=1; e->policy=EB_POLICY_DENY;
      CHECK(eb_network_classify(&n,99,6)==EB_DENY);
      CHECK(eb_network_resolve_action(eb_network_classify(&n,99,6),0,1)==EB_ACTION_BLOCK);
      CHECK(eb_network_resolve_action(eb_network_classify(&n,100,6),0,1)==EB_ACTION_PRESERVE);
      CHECK(eb_network_resolve_action(eb_network_classify(&n,99,6),0,0)==EB_ACTION_PRESERVE);
      CHECK(eb_network_resolve_action(EB_PENDING_DENY,0,1)==EB_ACTION_BLOCK);
      CHECK(eb_network_resolve_action(EB_DENY,1,0)==EB_ACTION_BLOCK);
      CHECK(eb_network_resolve_action(EB_PENDING_DENY,1,0)==EB_ACTION_BLOCK);
      CHECK(eb_network_resolve_action(EB_HOST,1,1)==EB_ACTION_CONTINUE);
      CHECK(eb_network_resolve_action(EB_HOST,0,0)==EB_ACTION_PRESERVE);
    }
    { PSID sid=NULL; PSECURITY_DESCRIPTOR sd=NULL; PACL acl=NULL; BOOL present=FALSE,defaulted=FALSE; PVOID ace=NULL;
      CHECK(ConvertStringSidToSidW(EB_SERVICE_SID,&sid)); CHECK(GetLengthSid(sid)==sizeof(eb_service_sid)); CHECK(memcmp(sid,eb_service_sid,sizeof(eb_service_sid))==0);
      CHECK(ConvertStringSecurityDescriptorToSecurityDescriptorW(EB_DEVICE_SDDL,SDDL_REVISION_1,&sd,NULL));
      CHECK(GetSecurityDescriptorDacl(sd,&present,&acl,&defaulted)); CHECK(present && acl && acl->AceCount==1);
      CHECK(GetAce(acl,0,&ace)); CHECK(((PACE_HEADER)ace)->AceType==ACCESS_ALLOWED_ACE_TYPE);
      CHECK(EqualSid(&((ACCESS_ALLOWED_ACE*)ace)->SidStart,sid));
      LocalFree(sd); LocalFree(sid);
    }
    memset(owner,1,32); memset(other,7,32); eb_initialize(&t); wire(p);
    CHECK(eb_decode(p,sizeof(p),&m)==EB_OK); original=m;
    CHECK(eb_decode(p,sizeof(p)-1,&m)==EB_INVALID_WIRE);
    CHECK(memcmp(&m,&original,sizeof(m))==0);
    CHECK(eb_decode(p,sizeof(p)+1,&m)==EB_INVALID_WIRE);
    CHECK(eb_decode(NULL,sizeof(p),&m)==EB_INVALID_WIRE);
    for(i=0;i<8;i++) { p[128+i]=1; CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); p[128+i]=0; }
    put32(p,2); CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); wire(p);
    put32(p+4,0); CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); wire(p);
    put32(p+8,3); CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); wire(p);
    put32(p+12,2); CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); wire(p);
    put64(p+24,0); CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); wire(p);
    put64(p+16,0); CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); wire(p);
    memset(p+32,0,32); CHECK(eb_decode(p,sizeof(p),&m)==EB_INVALID_WIRE); wire(p);
    CHECK(eb_decode(p,sizeof(p),&m)==EB_OK);
    CHECK(eb_connect(&t,owner,1,0)==EB_UNAUTHORIZED);
    CHECK(eb_connect(&t,owner,0,1)==EB_UNAUTHORIZED);
    CHECK(eb_connect(&t,owner,1,1)==EB_OK);
    { eb_identity invalid={0,1000}; CHECK(eb_mark_pending(&t,1,invalid)==EB_UNAUTHORIZED); CHECK(eb_classify(&t,invalid,4,1,6)==EB_DECISION_UNSUPPORTED); }
    CHECK(eb_connect(&t,other,2,1)==EB_UNAUTHORIZED);
    CHECK(eb_connect(&t,owner,2,1)==EB_CONFLICT);
    CHECK(eb_classify(&t,host,4,1,6)==EB_HOST);
    CHECK(eb_apply(&t,1,&m,a)==EB_NOT_FOUND);
    CHECK(eb_mark_pending(&t,1,a)==EB_OK);
    CHECK(eb_classify(&t,a,4,1,6)==EB_PENDING_DENY);
    CHECK(eb_mark_pending(&t,1,a)==EB_OK);
    before=t; m.owner[0]=9; CHECK(eb_apply(&t,1,&m,a)==EB_UNAUTHORIZED); CHECK(memcmp(&t,&before,sizeof(t))==0); m=original;
    m.policy=2; CHECK(eb_apply(&t,1,&m,a)==EB_INVALID_WIRE); CHECK(memcmp(&t,&before,sizeof(t))==0); m=original;
    m.operation=3; CHECK(eb_apply(&t,1,&m,a)==EB_INVALID_WIRE); CHECK(memcmp(&t,&before,sizeof(t))==0); m=original;
    m.generation=2; CHECK(eb_apply(&t,1,&m,a)==EB_STALE_GENERATION); CHECK(memcmp(&t,&before,sizeof(t))==0); m=original;
    CHECK(eb_apply(&t,1,&m,a)==EB_OK); CHECK(eb_apply(&t,1,&m,a)==EB_OK);
    CHECK(eb_classify(&t,a,4,1,6)==EB_DENY); CHECK(eb_classify(&t,a,4,1,17)==EB_DENY);
    before=t; m.policy=EB_POLICY_HOST; CHECK(eb_apply(&t,1,&m,a)==EB_CONFLICT); CHECK(memcmp(&t,&before,sizeof(t))==0); m=original;
    m.digest[0]++; CHECK(eb_apply(&t,1,&m,a)==EB_CONFLICT); m=original;
    m.container[0]++; CHECK(eb_apply(&t,1,&m,a)==EB_CONFLICT); m=original;
    m.instance[0]++; CHECK(eb_apply(&t,1,&m,a)==EB_CONFLICT); m=original;
    CHECK(eb_mark_pending(&t,1,b)==EB_OK); m.policy=EB_POLICY_HOST; m.instance[0]=8;
    CHECK(eb_apply(&t,1,&m,b)==EB_OK); CHECK(eb_classify(&t,b,4,1,17)==EB_HOST); m=original;
    CHECK(eb_classify(&t,a,6,1,6)==EB_DECISION_UNSUPPORTED);
    CHECK(eb_classify(&t,a,4,0,6)==EB_DECISION_UNSUPPORTED);
    CHECK(eb_classify(&t,a,4,1,1)==EB_DECISION_UNSUPPORTED);
    CHECK(eb_disconnect(&t,0)==EB_STALE_GENERATION); CHECK(eb_disconnect(&t,1)==EB_OK);
    CHECK(eb_classify(&t,a,4,1,6)==EB_DENY); CHECK(eb_classify(&t,b,4,1,6)==EB_HOST);
    CHECK(eb_mark_pending(&t,1,reused)==EB_DISCONNECTED); CHECK(eb_apply(&t,1,&m,a)==EB_DISCONNECTED);
    CHECK(eb_connect(&t,other,2,1)==EB_UNAUTHORIZED); CHECK(eb_connect(&t,owner,1,1)==EB_STALE_GENERATION);
    CHECK(eb_connect(&t,owner,2,0)==EB_UNAUTHORIZED); CHECK(eb_connect(&t,owner,2,1)==EB_OK);
    CHECK(eb_apply(&t,1,&m,a)==EB_STALE_GENERATION); m.generation=2;
    CHECK(eb_apply(&t,2,&m,a)==EB_OK); m.operation=EB_POLICY_REVOKE;
    CHECK(eb_apply(&t,2,&m,a)==EB_OK); CHECK(eb_classify(&t,a,4,1,6)==EB_PENDING_DENY);
    CHECK(eb_apply(&t,2,&m,a)==EB_OK); m.operation=EB_POLICY_BIND;
    CHECK(eb_apply(&t,2,&m,a)==EB_CONFLICT);
    CHECK(eb_process_exit(&t,reused)==EB_NOT_FOUND); CHECK(eb_classify(&t,a,4,1,6)==EB_PENDING_DENY);
    CHECK(eb_mark_pending(&t,2,reused)==EB_OK); CHECK(eb_process_exit(&t,a)==EB_OK);
    CHECK(eb_classify(&t,reused,4,1,6)==EB_PENDING_DENY); CHECK(eb_process_exit(&t,a)==EB_NOT_FOUND);
    CHECK(eb_process_exit(&t,reused)==EB_OK); CHECK(eb_process_exit(&t,b)==EB_OK);
    for(i=0;i<EB_POLICY_CAPACITY;i++) { eb_identity x={1000+i,5000+i}; CHECK(eb_mark_pending(&t,2,x)==EB_OK); }
    before=t; CHECK(eb_mark_pending(&t,2,host)==EB_CAPACITY); CHECK(memcmp(&t,&before,sizeof(t))==0);
    { eb_identity x={1000,5000}; CHECK(eb_process_exit(&t,x)==EB_OK); CHECK(eb_mark_pending(&t,2,host)==EB_OK); }
    printf("POLICY_FIXTURE_PASS checks=%u capacity=%u pointer_bits=%u\n",checks,EB_POLICY_CAPACITY,(unsigned)(sizeof(void*)*8)); return 0;
}
