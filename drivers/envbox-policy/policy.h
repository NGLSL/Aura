#ifndef ENVBOX_POLICY_H
#define ENVBOX_POLICY_H
#include <stdint.h>
#include <stddef.h>

/* All entry points require the adapter's exclusive lock. No allocation, OS
 * calls, borrowed references or pointers occur on the wire. The adapter owns
 * an actual referenced process object for every occupied slot. */
#define EB_POLICY_CAPACITY 64u
#define EB_POLICY_WIRE_SIZE 136u
#define EB_POLICY_VERSION 1u
#define EB_POLICY_BIND 1u
#define EB_POLICY_REVOKE 2u
#define EB_POLICY_HOST 0u
#define EB_POLICY_DENY 1u

/* Byte-array view only, never a cast to native integers or a decoded message.
 * These compile-time assertions hold on x64 and WOW64 without #pragma pack. */
typedef struct eb_wire_v1 {
    uint8_t version[4], length[4], operation[4], policy[4];
    uint8_t generation[8], process_handle[8], owner[32];
    uint8_t container[16], instance[16], digest[32], reserved[8];
} eb_wire_v1;
#define EB_WIRE_ASSERT(name, expr) typedef char eb_wire_assert_##name[(expr) ? 1 : -1]
EB_WIRE_ASSERT(size, sizeof(eb_wire_v1)==136);
EB_WIRE_ASSERT(version, offsetof(eb_wire_v1,version)==0);
EB_WIRE_ASSERT(length, offsetof(eb_wire_v1,length)==4);
EB_WIRE_ASSERT(operation, offsetof(eb_wire_v1,operation)==8);
EB_WIRE_ASSERT(policy, offsetof(eb_wire_v1,policy)==12);
EB_WIRE_ASSERT(generation, offsetof(eb_wire_v1,generation)==16);
EB_WIRE_ASSERT(process_handle, offsetof(eb_wire_v1,process_handle)==24);
EB_WIRE_ASSERT(owner, offsetof(eb_wire_v1,owner)==32);
EB_WIRE_ASSERT(container, offsetof(eb_wire_v1,container)==64);
EB_WIRE_ASSERT(instance, offsetof(eb_wire_v1,instance)==80);
EB_WIRE_ASSERT(digest, offsetof(eb_wire_v1,digest)==96);
EB_WIRE_ASSERT(reserved, offsetof(eb_wire_v1,reserved)==128);
#undef EB_WIRE_ASSERT

typedef enum eb_result {
    EB_OK, EB_INVALID_WIRE, EB_UNAUTHORIZED, EB_DISCONNECTED,
    EB_STALE_GENERATION, EB_CONFLICT, EB_CAPACITY, EB_NOT_FOUND,
    EB_UNSUPPORTED
} eb_result;
typedef enum eb_decision { EB_HOST, EB_DENY, EB_PENDING_DENY,
                           EB_DECISION_UNSUPPORTED } eb_decision;
typedef struct eb_identity { uint64_t process_key, creation_time; } eb_identity;
typedef struct eb_message {
    uint32_t operation, policy;
    uint64_t generation, process_handle;
    uint8_t owner[32], container[16], instance[16], digest[32];
} eb_message;
typedef struct eb_slot {
    eb_identity identity;
    uint8_t occupied, bound;
    uint32_t policy;
    uint8_t container[16], instance[16], digest[32];
} eb_slot;
typedef struct eb_table {
    uint8_t owner[32];
    uint64_t generation;
    uint8_t initialized, connected;
    eb_slot slots[EB_POLICY_CAPACITY];
} eb_table;

#ifdef __cplusplus
extern "C" {
#endif

void eb_initialize(eb_table *table);
/* verified must come from service-SID/token/connection validation, never wire.
 * Reconnect requires the same owner and a strictly newer generation. */
eb_result eb_connect(eb_table *, const uint8_t owner[32], uint64_t generation,
                     int verified);
eb_result eb_disconnect(eb_table *, uint64_t authenticated_generation);
eb_result eb_decode(const void *wire, size_t length, eb_message *out);
/* Must run while target is still held behind the launch gate. Failure means
 * adapter must not release the target. A zero/reused PID is never an identity. */
eb_result eb_mark_pending(eb_table *, uint64_t authenticated_generation,
                          eb_identity referenced_identity);
/* Adapter first decodes, validates owner/generation, then resolves handle with
 * ObReferenceObjectByHandle(UserMode, *PsProcessType), checks rights/ownership
 * and creation time. Identity must be derived from that reference. */
eb_result eb_authorize(const eb_table *, uint64_t authenticated_generation,
                       const eb_message *);
eb_result eb_apply(eb_table *, uint64_t authenticated_generation,
                   const eb_message *, eb_identity referenced_identity);
/* Only process-exit callback with retained object + creation time may remove.
 * Revoke retains Pending/Deny; disconnect retains every existing binding. */
eb_result eb_process_exit(eb_table *, eb_identity referenced_identity);
/* Supported scope: IPv4 (4), outbound (1), TCP (6) or UDP (17), loopback too.
 * Unsupported scope is distinct and must not be translated into permit. */
eb_decision eb_classify(const eb_table *, eb_identity referenced_identity,
                        uint32_t family, uint32_t direction, uint32_t protocol);
#ifdef __cplusplus
}
#endif
#endif
