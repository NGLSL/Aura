#pragma once
#include <windows.h>
struct CleanupFixtureReport {
  DWORD pid;
  DWORD process_closes;
  DWORD thread_closes;
  DWORD terminate_calls;
  DWORD fault_error;
  DWORD wait_result;
  DWORD exit_code;
  DWORD forced_cleanup;
};
// Modes: 0 = success/control, 1 = dead-child injection, 2 = query-only
// ResumeThread, 3 = invalid Job assignment. Only fixture-owned children match.
using CleanupFixtureSetModeFn = void (*)(int);
using CleanupFixtureFinishFn = CleanupFixtureReport (*)();
