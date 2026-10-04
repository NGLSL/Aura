#pragma once

// Called inside the existing Detours transaction. This only installs an
// application-entry detour; it never waits for management under loader lock.
int EnvBoxInstallStartupGate();
