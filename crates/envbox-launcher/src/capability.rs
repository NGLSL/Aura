//! Process-level capability probe (packaged-v1 ticket 38).
//!
//! Selection is data-driven via [`TargetCapabilities`] + [`InjectionCapability`].
//! Policy lives in `envbox_core::session::evaluate_injection_support` — no bypass.

use envbox_core::{
    evaluate_injection_support, InjectionCapability, IntegrityLevel, MitigationPolicy,
    TargetCapabilities,
};

/// Trust / packaging / architecture signals collected before attach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessProbe {
    pub is_app_container: bool,
    pub integrity: IntegrityLevel,
    pub signature_policy: MitigationPolicy,
    pub dynamic_code_policy: MitigationPolicy,
    pub image_load_policy: MitigationPolicy,
    pub can_open_process: bool,
    /// Target PE architecture (from probe / PE header).
    pub architecture: &'static str,
    /// Process type label (win32 / packaged_win32 / appcontainer / unknown).
    pub process_type: &'static str,
}

impl ProcessProbe {
    /// Conservative default when the process cannot be opened or queried.
    pub fn unqueryable() -> Self {
        Self {
            is_app_container: false,
            integrity: IntegrityLevel::Unknown,
            signature_policy: MitigationPolicy::Unknown,
            dynamic_code_policy: MitigationPolicy::Unknown,
            image_load_policy: MitigationPolicy::Unknown,
            can_open_process: false,
            architecture: "unknown",
            process_type: "unknown",
        }
    }

    /// Medium-IL desktop process with no blocking mitigations.
    pub fn medium_clean() -> Self {
        Self {
            is_app_container: false,
            integrity: IntegrityLevel::Medium,
            signature_policy: MitigationPolicy::Allow,
            dynamic_code_policy: MitigationPolicy::Allow,
            image_load_policy: MitigationPolicy::Allow,
            can_open_process: true,
            architecture: "x64",
            process_type: "win32",
        }
    }

    pub fn integrity_ok(&self) -> bool {
        matches!(
            self.integrity,
            IntegrityLevel::Medium | IntegrityLevel::High | IntegrityLevel::System
        )
    }

    pub fn to_injection_capability(&self, is_packaged: bool) -> InjectionCapability {
        let (supported, reason) = evaluate_injection_support(
            self.is_app_container,
            self.signature_policy,
            self.dynamic_code_policy,
            self.image_load_policy,
            self.integrity_ok(),
            self.can_open_process,
        );
        InjectionCapability {
            is_packaged,
            is_app_container: self.is_app_container,
            integrity_level: self.integrity,
            signature_policy: self.signature_policy,
            dynamic_code_policy: self.dynamic_code_policy,
            image_load_policy: self.image_load_policy,
            supported,
            reason,
        }
    }
}

/// Classic Win32 root we create ourselves: full pre-execution path.
pub fn win32_capabilities() -> TargetCapabilities {
    TargetCapabilities::win32()
}

/// Derive capabilities after activation (packaged root / child attach).
///
/// `can_suspend` is false for already-running packaged roots (PostActivation).
pub fn capabilities_after_probe(
    probe: &ProcessProbe,
    injection_supported: bool,
) -> TargetCapabilities {
    TargetCapabilities {
        can_suspend: false,
        can_inject_runtime: injection_supported && probe.can_open_process,
        can_create_environment_block: false,
        can_assign_job: !probe.is_app_container,
        can_track_children: true,
    }
}

/// Map mitigation `Flags` bits (PROCESS_MITIGATION_*_POLICY) to policy.
///
/// Bit layouts match winnt.h DUMMYSTRUCTNAME fields (low bit first).
fn signature_from_flags(flags: u32) -> MitigationPolicy {
    // bit0 MicrosoftSignedOnly, bit1 StoreSignedOnly
    if flags & 0b11 != 0 {
        MitigationPolicy::Blocking
    } else if flags != 0 {
        MitigationPolicy::Off
    } else {
        MitigationPolicy::Allow
    }
}

fn dynamic_from_flags(flags: u32) -> MitigationPolicy {
    // bit0 ProhibitDynamicCode
    if flags & 0b1 != 0 {
        MitigationPolicy::Blocking
    } else if flags != 0 {
        MitigationPolicy::Off
    } else {
        MitigationPolicy::Allow
    }
}

fn image_load_from_flags(flags: u32) -> MitigationPolicy {
    // bit0 NoRemoteImages, bit1 NoLowMandatoryLabelImages
    if flags & 0b11 != 0 {
        MitigationPolicy::Blocking
    } else if flags != 0 {
        MitigationPolicy::Off
    } else {
        MitigationPolicy::Allow
    }
}

fn integrity_from_rid(rid: u32) -> IntegrityLevel {
    const LOW: u32 = 0x1000;
    const MEDIUM: u32 = 0x2000;
    const HIGH: u32 = 0x3000;
    const SYSTEM: u32 = 0x4000;
    if rid < LOW {
        IntegrityLevel::Low
    } else if rid < MEDIUM {
        IntegrityLevel::Low
    } else if rid < HIGH {
        IntegrityLevel::Medium
    } else if rid < SYSTEM {
        IntegrityLevel::High
    } else {
        IntegrityLevel::System
    }
}

/// Live process probe (Windows). On non-Windows returns unqueryable.
pub fn probe_pid(pid: u32) -> ProcessProbe {
    #[cfg(windows)]
    {
        win::probe_process(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        ProcessProbe::unqueryable()
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenIntegrityLevel, TokenIsAppContainer, TOKEN_QUERY,
    };
    use windows::Win32::System::SystemServices::{
        PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY, PROCESS_MITIGATION_DYNAMIC_CODE_POLICY,
        PROCESS_MITIGATION_IMAGE_LOAD_POLICY,
    };
    use windows::Win32::System::Threading::{
        GetProcessMitigationPolicy, OpenProcess, OpenProcessToken, ProcessDynamicCodePolicy,
        ProcessImageLoadPolicy, ProcessSignaturePolicy, PROCESS_QUERY_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }

    pub fn probe_process(pid: u32) -> ProcessProbe {
        unsafe {
            let access = PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION;
            let Ok(raw) = OpenProcess(access, false, pid) else {
                return ProcessProbe::unqueryable();
            };
            let process = OwnedHandle(raw);

            let mut is_app_container = false;
            let mut integrity = IntegrityLevel::Unknown;

            let mut token = HANDLE::default();
            if OpenProcessToken(process.0, TOKEN_QUERY, &mut token).is_ok() {
                let token = OwnedHandle(token);

                let mut ac = 0u32;
                let mut ret = 0u32;
                if GetTokenInformation(
                    token.0,
                    TokenIsAppContainer,
                    Some(&mut ac as *mut u32 as *mut _),
                    std::mem::size_of::<u32>() as u32,
                    &mut ret,
                )
                .is_ok()
                {
                    is_app_container = ac != 0;
                }

                // TOKEN_MANDATORY_LABEL → SID last subauthority = integrity RID.
                let mut buf = vec![0u8; 64];
                let mut ret = 0u32;
                if GetTokenInformation(
                    token.0,
                    TokenIntegrityLevel,
                    Some(buf.as_mut_ptr() as *mut _),
                    buf.len() as u32,
                    &mut ret,
                )
                .is_ok()
                    && (ret as usize) >= std::mem::size_of::<usize>() + std::mem::size_of::<u32>()
                {
                    // TOKEN_MANDATORY_LABEL begins with SID_AND_ATTRIBUTES:
                    // { Sid: *mut SID, Attributes: u32 }.  Its size is 8
                    // bytes on Win32 and 16 bytes on Win64; the pointer is
                    // therefore decoded using the target process width.
                    let sid_ptr = match std::mem::size_of::<usize>() {
                        4 => u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize,
                        8 => u64::from_le_bytes(buf[0..8].try_into().unwrap()) as usize,
                        _ => 0,
                    };
                    if sid_ptr != 0 {
                        let sid = sid_ptr as *const u8;
                        let sub_count = *sid.add(1) as usize;
                        if sub_count >= 1 {
                            // SubAuthority array starts at offset 8.
                            let rid_off = 8 + (sub_count - 1) * 4;
                            let rid = u32::from_le_bytes(
                                std::slice::from_raw_parts(sid.add(rid_off), 4)
                                    .try_into()
                                    .unwrap_or([0; 4]),
                            );
                            integrity = integrity_from_rid(rid);
                        }
                    }
                }
            }

            let mut signature = PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY::default();
            let mut dynamic = PROCESS_MITIGATION_DYNAMIC_CODE_POLICY::default();
            let mut image = PROCESS_MITIGATION_IMAGE_LOAD_POLICY::default();
            // Query failure must stay Unknown (fail closed), never Allow.
            let sig_ok = GetProcessMitigationPolicy(
                process.0,
                ProcessSignaturePolicy,
                &mut signature as *mut _ as *mut _,
                std::mem::size_of_val(&signature),
            )
            .is_ok();
            let dyn_ok = GetProcessMitigationPolicy(
                process.0,
                ProcessDynamicCodePolicy,
                &mut dynamic as *mut _ as *mut _,
                std::mem::size_of_val(&dynamic),
            )
            .is_ok();
            let img_ok = GetProcessMitigationPolicy(
                process.0,
                ProcessImageLoadPolicy,
                &mut image as *mut _ as *mut _,
                std::mem::size_of_val(&image),
            )
            .is_ok();

            let sig_flags = signature.Anonymous.Flags;
            let dyn_flags = dynamic.Anonymous.Flags;
            let img_flags = image.Anonymous.Flags;

            ProcessProbe {
                is_app_container,
                integrity,
                signature_policy: if sig_ok {
                    signature_from_flags(sig_flags)
                } else {
                    MitigationPolicy::Unknown
                },
                dynamic_code_policy: if dyn_ok {
                    dynamic_from_flags(dyn_flags)
                } else {
                    MitigationPolicy::Unknown
                },
                image_load_policy: if img_ok {
                    image_load_from_flags(img_flags)
                } else {
                    MitigationPolicy::Unknown
                },
                can_open_process: true,
                architecture: process_arch(process.0),
                process_type: if is_app_container {
                    "appcontainer"
                } else {
                    "win32"
                },
            }
        }
    }
}

/// Target architecture from Wow64 state (x64 / x86 / unknown).
#[cfg(windows)]
fn process_arch(process: windows::Win32::Foundation::HANDLE) -> &'static str {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::System::Threading::IsWow64Process;
    let mut wow = BOOL(0);
    unsafe {
        if IsWow64Process(process, &mut wow).is_ok() {
            if wow.as_bool() {
                "x86"
            } else {
                "x64"
            }
        } else {
            "unknown"
        }
    }
}

#[cfg(not(windows))]
fn process_arch(_process: ()) -> &'static str {
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::MitigationPolicy;

    #[test]
    fn medium_clean_probe_supports_injection() {
        let probe = ProcessProbe::medium_clean();
        let inj = probe.to_injection_capability(false);
        assert!(inj.supported);
        assert!(inj.reason.is_none());
    }

    #[test]
    fn app_container_probe_rejects() {
        let mut probe = ProcessProbe::medium_clean();
        probe.is_app_container = true;
        let inj = probe.to_injection_capability(true);
        assert!(!inj.supported);
        assert!(inj.reason.unwrap().contains("AppContainer"));
    }

    #[test]
    fn unqueryable_probe_rejects() {
        let probe = ProcessProbe::unqueryable();
        let inj = probe.to_injection_capability(false);
        assert!(!inj.supported);
    }

    #[test]
    fn blocking_signature_maps_to_reject() {
        let mut probe = ProcessProbe::medium_clean();
        probe.signature_policy = MitigationPolicy::Blocking;
        assert!(!probe.to_injection_capability(true).supported);
    }

    #[test]
    fn capabilities_after_probe_post_activation_shape() {
        let probe = ProcessProbe::medium_clean();
        let caps = capabilities_after_probe(&probe, true);
        assert!(!caps.can_suspend);
        assert!(caps.can_inject_runtime);
        assert!(!caps.can_create_environment_block);
    }

    #[test]
    fn mitigation_flag_bits_map_to_blocking() {
        assert_eq!(signature_from_flags(1), MitigationPolicy::Blocking);
        assert_eq!(signature_from_flags(2), MitigationPolicy::Blocking);
        assert_eq!(signature_from_flags(0), MitigationPolicy::Allow);
        assert_eq!(dynamic_from_flags(1), MitigationPolicy::Blocking);
        assert_eq!(image_load_from_flags(1), MitigationPolicy::Blocking);
    }

    #[test]
    fn integrity_rid_buckets() {
        assert_eq!(integrity_from_rid(0x1000), IntegrityLevel::Low);
        assert_eq!(integrity_from_rid(0x2000), IntegrityLevel::Medium);
        assert_eq!(integrity_from_rid(0x3000), IntegrityLevel::High);
        assert_eq!(integrity_from_rid(0x4000), IntegrityLevel::System);
    }
}
