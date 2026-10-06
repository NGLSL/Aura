#pragma once
#include <windows.h>

// Missing means legacy; any present value other than exact "1" is invalid.
// INIT_ONCE freezes the decision before the first connection.
bool EnvBoxServiceBootstrapConfigurationValid();
bool EnvBoxServiceBootstrapRequired();
bool EnvBoxValidateTrustedServicePipe(HANDLE pipe);
constexpr DWORD kEnvBoxPipeClientAccess = FILE_READ_DATA | FILE_WRITE_DATA |
    FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE;
