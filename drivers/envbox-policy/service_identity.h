#ifndef ENVBOX_POLICY_SERVICE_IDENTITY_H
#define ENVBOX_POLICY_SERVICE_IDENTITY_H
/* Generated from read-only `sc.exe showsid AuraPolicyService`. Build script
 * verifies both the exact canonical string and these 32 binary SID bytes. */
#define EB_SERVICE_NAME L"AuraPolicyService"
#define EB_SERVICE_SID L"S-1-5-80-3820527054-1736232212-1554131609-2082016452-471345742"
#define EB_DEVICE_SDDL L"D:P(A;;GA;;;S-1-5-80-3820527054-1736232212-1554131609-2082016452-471345742)"
static const unsigned char eb_service_sid[32] = {
    0x01,0x06,0x00,0x00,0x00,0x00,0x00,0x05,
    0x50,0x00,0x00,0x00,0xCE,0x9D,0xB8,0xE3,
    0x14,0xCD,0x7C,0x67,0x99,0x2A,0xA2,0x5C,
    0xC4,0x0C,0x19,0x7C,0x4E,0x2A,0x18,0x1C
};
#endif
