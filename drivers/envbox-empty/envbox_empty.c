/*
 * EnvBox empty WDM build fixture.
 *
 * This module deliberately has no device object, device/event/IO callbacks,
 * IOCTLs, or policy logic.  Its required DriverUnload routine is the only
 * callback assigned by DriverEntry.  It exists only to exercise the isolated
 * WDK build and package-signing inspection path.  It is never installed or
 * loaded by the build script.
 */
#include <ntddk.h>

DRIVER_UNLOAD EnvBoxEmptyUnload;

_Use_decl_annotations_
VOID EnvBoxEmptyUnload(_In_ PDRIVER_OBJECT DriverObject)
{
    UNREFERENCED_PARAMETER(DriverObject);
}

_Use_decl_annotations_
NTSTATUS DriverEntry(PDRIVER_OBJECT DriverObject, PUNICODE_STRING RegistryPath)
{
    UNREFERENCED_PARAMETER(RegistryPath);

    DriverObject->DriverUnload = EnvBoxEmptyUnload;
    return STATUS_SUCCESS;
}
