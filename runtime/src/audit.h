// Audit Mode sink (ticket 20). Best-effort JSONL append; Fail Open on any error.
// Never logs file contents, tokens, full env blocks, or command bodies.

#pragma once

#include "runtime_profile.h"

// Open <config_root>/audit/<instance_id>.jsonl when audit is enabled.
// Writes one EnvBoxAuditInit event on success. Never fatal.
void EnvBoxAuditInit(const RuntimeProfile* pfl);

// Append one Audit Event. No-op when audit is off or sink is unavailable.
void EnvBoxAuditEvent(const char* api, int virtualized, const char* summary);

// Close sink. Safe to call multiple times.
void EnvBoxAuditShutdown(void);
