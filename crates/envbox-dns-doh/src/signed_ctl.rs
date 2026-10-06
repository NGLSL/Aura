//! Controlled-input research gate. This does not authorize production AuthRoot.
//! Pins are explicit trust inputs; no signer-chain, rotation or revocation proof
//! is implied. Only attribute-free CTLs and RSA/SHA-2 signers are supported.
use crate::{budget::Budget, Error};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, ptr, slice};
use windows_sys::Win32::{Foundation::FILETIME, Security::Cryptography::*};

const ENCODING: u32 = X509_ASN_ENCODING | PKCS_7_ASN_ENCODING;
const ROOT_LIST: &[u8] = b"1.3.6.1.4.1.311.10.3.9";
const RSA: &[u8] = b"1.2.840.113549.1.1.1";
const SHA256: &[u8] = b"2.16.840.1.101.3.4.2.1";
const SHA1: &[u8] = b"1.3.14.3.2.26";

#[derive(Debug, PartialEq, Eq)]
pub enum CtlError {
    Budget(Error),
    Bounds,
    Decode,
    Signature,
    Signer,
    Algorithm,
    Policy,
    Time,
    Rollback,
}
impl From<Error> for CtlError {
    fn from(value: Error) -> Self {
        Self::Budget(value)
    }
}

pub struct Previous<'a> {
    /// Sequence belonging to `Policy::list_identifier`, little-endian unsigned.
    pub sequence: &'a [u8],
    pub encoded_sha256: [u8; 32],
}
pub struct Policy<'a> {
    pub list_identifier: &'a [u8],
    pub usage: &'a [u8],
    /// Explicit evaluation clock. This API is not a production trust decision.
    pub now_filetime: u64,
    pub previous: Option<Previous<'a>>,
}
#[derive(Debug)]
pub struct VerifiedCtl {
    pub list_identifier: Vec<u8>,
    pub sequence: Vec<u8>,
    pub encoded_sha256: [u8; 32],
    pub subject_identifiers: Vec<Vec<u8>>,
    pub signer_count: u32,
}

struct Store(HCERTSTORE);
impl Drop for Store {
    fn drop(&mut self) {
        unsafe {
            CertCloseStore(self.0, 0);
        }
    }
}
struct Ctl(*mut CTL_CONTEXT);
impl Drop for Ctl {
    fn drop(&mut self) {
        unsafe {
            CertFreeCTLContext(self.0);
        }
    }
}
struct Cert(*mut CERT_CONTEXT);
impl Drop for Cert {
    fn drop(&mut self) {
        unsafe {
            CertFreeCertificateContext(self.0);
        }
    }
}

fn time(value: FILETIME) -> u64 {
    u64::from(value.dwLowDateTime) | u64::from(value.dwHighDateTime) << 32
}
unsafe fn oid<'a>(value: *const u8) -> Result<&'a [u8], CtlError> {
    if value.is_null() {
        return Err(CtlError::Decode);
    }
    // API-owned decoded contexts have no caller-visible allocation extent.
    // Walk only the supported maximum, never scan an unbounded C string.
    for length in 0..=128 {
        if unsafe { *value.add(length) } == 0 {
            return Ok(unsafe { slice::from_raw_parts(value, length) });
        }
    }
    Err(CtlError::Bounds)
}

#[derive(Clone, Copy)]
struct Region {
    start: usize,
    len: usize,
}
impl Region {
    fn returned(
        start: *const u8,
        actual: usize,
        allocated: usize,
        header: usize,
    ) -> Result<Self, CtlError> {
        if start.is_null()
            || actual < header
            || actual > allocated
            || (start as usize).checked_add(actual).is_none()
        {
            return Err(CtlError::Bounds);
        }
        Ok(Self {
            start: start as usize,
            len: actual,
        })
    }
    fn contains(&self, pointer: *const u8, len: usize) -> bool {
        let address = pointer as usize;
        address >= self.start
            && address
                .checked_add(len)
                .is_some_and(|end| end <= self.start + self.len)
    }
    unsafe fn oid<'a>(&self, pointer: *const u8) -> Result<&'a [u8], CtlError> {
        if !self.contains(pointer, 1) {
            return Err(CtlError::Bounds);
        }
        let remaining = self.start + self.len - pointer as usize;
        for length in 0..remaining.min(129) {
            if unsafe { *pointer.add(length) } == 0 {
                return Ok(unsafe { slice::from_raw_parts(pointer, length) });
            }
        }
        Err(CtlError::Bounds)
    }
    unsafe fn blob<'a>(
        &self,
        value: CRYPT_INTEGER_BLOB,
        limit: usize,
    ) -> Result<&'a [u8], CtlError> {
        if value.cbData as usize > limit
            || (value.cbData != 0 && !self.contains(value.pbData, value.cbData as usize))
        {
            return Err(CtlError::Bounds);
        }
        unsafe { blob(value, limit) }
    }
    unsafe fn algorithm<'a>(
        &self,
        value: &CRYPT_ALGORITHM_IDENTIFIER,
    ) -> Result<&'a [u8], CtlError> {
        let parameters = unsafe { self.blob(value.Parameters, 2)? };
        if !parameters.is_empty() && parameters != [5, 0] {
            return Err(CtlError::Algorithm);
        }
        unsafe { self.oid(value.pszObjId) }
    }
}
unsafe fn blob<'a>(value: CRYPT_INTEGER_BLOB, limit: usize) -> Result<&'a [u8], CtlError> {
    let len = value.cbData as usize;
    if len > limit || (len > 0 && value.pbData.is_null()) {
        return Err(CtlError::Bounds);
    }
    if len == 0 {
        return Ok(&[]);
    }
    Ok(unsafe { slice::from_raw_parts(value.pbData, len) })
}
fn normalized(value: &[u8]) -> &[u8] {
    let end = value.iter().rposition(|v| *v != 0).map_or(0, |i| i + 1);
    &value[..end]
}
fn compare(a: &[u8], b: &[u8]) -> Ordering {
    let (a, b) = (normalized(a), normalized(b));
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}
fn strong_digest(value: &[u8]) -> bool {
    [SHA256, b"2.16.840.1.101.3.4.2.2", b"2.16.840.1.101.3.4.2.3"].contains(&value)
}
unsafe fn plain_algorithm(value: &CRYPT_ALGORITHM_IDENTIFIER) -> Result<(), CtlError> {
    // Supported RSA PKCS#1 and digest OIDs have absent or ASN.1 NULL parameters.
    let parameters = unsafe { blob(value.Parameters, 2)? };
    if !parameters.is_empty() && parameters != [5, 0] {
        return Err(CtlError::Algorithm);
    }
    Ok(())
}

unsafe fn signer_policy(
    cert: *const CERT_CONTEXT,
    now: u64,
    budget: Budget,
) -> Result<(), CtlError> {
    budget.check()?;
    let info = unsafe { (*cert).pCertInfo.as_ref() }.ok_or(CtlError::Decode)?;
    if now < time(info.NotBefore) || now >= time(info.NotAfter) {
        return Err(CtlError::Time);
    }
    let public = &info.SubjectPublicKeyInfo;
    unsafe {
        plain_algorithm(&public.Algorithm)?;
        plain_algorithm(&info.SignatureAlgorithm)?;
    }
    let key_bits = unsafe { CertGetPublicKeyLength(ENCODING, public) };
    budget.check()?;
    if unsafe { oid(public.Algorithm.pszObjId)? } != RSA
        || key_bits < 2048
        || ![
            b"1.2.840.113549.1.1.11".as_slice(),
            b"1.2.840.113549.1.1.12",
            b"1.2.840.113549.1.1.13",
        ]
        .contains(&unsafe { oid(info.SignatureAlgorithm.pszObjId)? })
    {
        return Err(CtlError::Algorithm);
    }
    let mut size = 0;
    let result = unsafe {
        CertGetEnhancedKeyUsage(
            cert,
            CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG,
            ptr::null_mut(),
            &mut size,
        )
    };
    budget.check()?;
    if result == 0 || size as usize > 4096 || (size as usize) < size_of::<CTL_USAGE>() {
        return Err(CtlError::Signer);
    }
    let allocated = size as usize;
    let mut aligned = vec![0usize; allocated.div_ceil(size_of::<usize>())];
    let result = unsafe {
        CertGetEnhancedKeyUsage(
            cert,
            CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG,
            aligned.as_mut_ptr().cast(),
            &mut size,
        )
    };
    budget.check()?;
    if result == 0 {
        return Err(CtlError::Signer);
    }
    let region = Region::returned(
        aligned.as_ptr().cast(),
        size as usize,
        allocated,
        size_of::<CTL_USAGE>(),
    )?;
    let usage = unsafe { &*aligned.as_ptr().cast::<CTL_USAGE>() };
    if usage.cUsageIdentifier == 0
        || usage.cUsageIdentifier > 16
        || usage.rgpszUsageIdentifier.is_null()
    {
        return Err(CtlError::Signer);
    }
    let pointers_size = (usage.cUsageIdentifier as usize)
        .checked_mul(size_of::<*mut u8>())
        .ok_or(CtlError::Bounds)?;
    if !region.contains(usage.rgpszUsageIdentifier.cast(), pointers_size) {
        return Err(CtlError::Bounds);
    }
    let mut root_list = false;
    for index in 0..usage.cUsageIdentifier as usize {
        let pointer = unsafe { usage.rgpszUsageIdentifier.add(index).read_unaligned() };
        root_list |= unsafe { region.oid(pointer)? } == ROOT_LIST;
    }
    if !root_list {
        return Err(CtlError::Signer);
    }
    Ok(())
}

/// Authenticate every signer before interpreting CTL authorization metadata.
/// All stores are process-memory only; no chain, URL, registry or key APIs run.
pub fn verify(
    encoded: &[u8],
    pins: &[&[u8]],
    policy: Policy<'_>,
    budget: Budget,
) -> Result<VerifiedCtl, CtlError> {
    budget.check()?;
    if encoded.is_empty()
        || encoded.len() > 4 * 1024 * 1024
        || pins.is_empty()
        || pins.len() > 8
        || policy.list_identifier.is_empty()
        || policy.list_identifier.len() > 128
        || policy.usage != ROOT_LIST
        || policy
            .previous
            .as_ref()
            .is_some_and(|p| p.sequence.is_empty() || p.sequence.len() > 16)
    {
        return Err(CtlError::Bounds);
    }
    let store = Store(unsafe {
        CertOpenStore(
            CERT_STORE_PROV_MEMORY,
            0,
            0,
            CERT_STORE_CREATE_NEW_FLAG,
            ptr::null(),
        )
    });
    budget.check()?;
    if store.0.is_null() {
        return Err(CtlError::Decode);
    }
    for pin in pins {
        budget.check()?;
        if pin.is_empty() || pin.len() > 64 * 1024 {
            return Err(CtlError::Bounds);
        }
        let result = unsafe {
            CertAddEncodedCertificateToStore(
                store.0,
                ENCODING,
                pin.as_ptr(),
                pin.len() as u32,
                CERT_STORE_ADD_ALWAYS,
                ptr::null_mut(),
            )
        };
        budget.check()?;
        if result == 0 {
            return Err(CtlError::Decode);
        }
    }
    let ctl =
        Ctl(unsafe { CertCreateCTLContext(ENCODING, encoded.as_ptr(), encoded.len() as u32) });
    budget.check()?;
    if ctl.0.is_null() {
        return Err(CtlError::Decode);
    }
    let msg = unsafe { (*ctl.0).hCryptMsg };
    if msg.is_null() {
        return Err(CtlError::Signature);
    }
    let mut count = 0u32;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        CryptMsgGetParam(
            msg,
            CMSG_SIGNER_COUNT_PARAM,
            0,
            (&mut count as *mut u32).cast(),
            &mut size,
        )
    };
    budget.check()?;
    if result == 0 || size != 4 || count == 0 || count > 8 {
        return Err(CtlError::Signature);
    }
    for index in 0..count {
        budget.check()?;
        let mut actual = index;
        let mut cert = ptr::null_mut();
        let result = unsafe {
            CryptMsgGetAndVerifySigner(
                msg,
                1,
                &store.0,
                CMSG_TRUSTED_SIGNER_FLAG | CMSG_USE_SIGNER_INDEX_FLAG,
                &mut cert,
                &mut actual,
            )
        };
        let cert = Cert(cert);
        budget.check()?;
        if result == 0 || cert.0.is_null() || actual != index {
            return Err(CtlError::Signature);
        }
        let der = unsafe {
            blob(
                CRYPT_INTEGER_BLOB {
                    cbData: (*cert.0).cbCertEncoded,
                    pbData: (*cert.0).pbCertEncoded,
                },
                64 * 1024,
            )?
        };
        if !pins.contains(&der) {
            return Err(CtlError::Signer);
        }
        unsafe {
            signer_policy(cert.0, policy.now_filetime, budget)?;
        }
        budget.check()?;
        let mut size = 0;
        let result = unsafe {
            CryptMsgGetParam(
                msg,
                CMSG_SIGNER_INFO_PARAM,
                index,
                ptr::null_mut(),
                &mut size,
            )
        };
        budget.check()?;
        if result == 0
            || size as usize > 64 * 1024
            || (size as usize) < size_of::<CMSG_SIGNER_INFO>()
        {
            return Err(CtlError::Decode);
        }
        let allocated = size as usize;
        let mut aligned = vec![0usize; allocated.div_ceil(size_of::<usize>())];
        let result = unsafe {
            CryptMsgGetParam(
                msg,
                CMSG_SIGNER_INFO_PARAM,
                index,
                aligned.as_mut_ptr().cast(),
                &mut size,
            )
        };
        budget.check()?;
        if result == 0 {
            return Err(CtlError::Decode);
        }
        let region = Region::returned(
            aligned.as_ptr().cast(),
            size as usize,
            allocated,
            size_of::<CMSG_SIGNER_INFO>(),
        )?;
        let signer = unsafe { &*aligned.as_ptr().cast::<CMSG_SIGNER_INFO>() };
        if !strong_digest(unsafe { region.algorithm(&signer.HashAlgorithm)? })
            || unsafe { region.algorithm(&signer.HashEncryptionAlgorithm)? } != RSA
        {
            return Err(CtlError::Algorithm);
        }
        // Unknown signed or unsigned attributes are unsupported in this stage.
        if signer.AuthAttrs.cAttr != 0 || signer.UnauthAttrs.cAttr != 0 {
            return Err(CtlError::Policy);
        }
    }
    let info = unsafe { (*ctl.0).pCtlInfo.as_ref() }.ok_or(CtlError::Decode)?;
    if info.dwVersion != 0
        || info.cExtension != 0
        || info.SubjectUsage.cUsageIdentifier != 1
        || info.SubjectUsage.rgpszUsageIdentifier.is_null()
    {
        return Err(CtlError::Policy);
    }
    if unsafe { oid(*info.SubjectUsage.rgpszUsageIdentifier)? } != policy.usage {
        return Err(CtlError::Policy);
    }
    let list = unsafe { blob(info.ListIdentifier, 128)? };
    if list != policy.list_identifier {
        return Err(CtlError::Policy);
    }
    let sequence = unsafe { blob(info.SequenceNumber, 16)? };
    if sequence.is_empty() {
        return Err(CtlError::Policy);
    }
    let hash: [u8; 32] = Sha256::digest(encoded).into();
    if let Some(previous) = policy.previous {
        match compare(sequence, previous.sequence) {
            Ordering::Less => return Err(CtlError::Rollback),
            Ordering::Equal if hash != previous.encoded_sha256 => return Err(CtlError::Rollback),
            _ => (),
        }
    }
    if time(info.ThisUpdate) == 0
        || time(info.ThisUpdate) > policy.now_filetime
        || time(info.NextUpdate) <= policy.now_filetime
        || time(info.NextUpdate) <= time(info.ThisUpdate)
    {
        return Err(CtlError::Time);
    }
    unsafe {
        plain_algorithm(&info.SubjectAlgorithm)?;
    }
    let width = match unsafe { oid(info.SubjectAlgorithm.pszObjId)? } {
        SHA1 => 20,
        SHA256 => 32,
        _ => return Err(CtlError::Algorithm),
    };
    if info.cCTLEntry == 0 || info.cCTLEntry > 4096 || info.rgCTLEntry.is_null() {
        return Err(CtlError::Bounds);
    }
    let entries = unsafe { slice::from_raw_parts(info.rgCTLEntry, info.cCTLEntry as usize) };
    let mut identifiers = Vec::with_capacity(entries.len());
    for entry in entries {
        budget.check()?;
        if entry.cAttribute != 0 {
            return Err(CtlError::Policy);
        }
        let id = unsafe { blob(entry.SubjectIdentifier, 32)? };
        if id.len() != width || identifiers.iter().any(|v: &Vec<u8>| v.as_slice() == id) {
            return Err(CtlError::Policy);
        }
        identifiers.push(id.to_vec());
    }
    budget.check()?;
    Ok(VerifiedCtl {
        list_identifier: list.to_vec(),
        sequence: sequence.to_vec(),
        encoded_sha256: hash,
        subject_identifiers: identifiers,
        signer_count: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    const SIGNER: &[u8] =
        include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/signer.der");
    const VALID: &[u8] =
        include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/valid.stl");
    const NOW: u64 = 134_357_184_000_000_000; // 2026-10-06 00:00 UTC
    fn policy() -> Policy<'static> {
        Policy {
            list_identifier: b"Aura research v1",
            usage: ROOT_LIST,
            now_filetime: NOW,
            previous: None,
        }
    }
    #[test]
    fn returned_buffer_ranges_use_actual_length() {
        let mut buffer = [b'x'; 64];
        buffer[12..16].copy_from_slice(b"1.2\0");
        let start = buffer.as_ptr();
        assert!(Region::returned(start, 7, 64, 8).is_err());
        assert!(Region::returned(start, 65, 64, 8).is_err());
        let region = Region::returned(start, 32, 64, 8).unwrap();
        assert!(region.contains(unsafe { start.add(8) }, 8));
        assert!(!region.contains(unsafe { start.add(30) }, 4));
        assert!(!region.contains(usize::MAX as *const u8, 8));
        assert_eq!(unsafe { region.oid(start.add(12)) }.unwrap(), b"1.2");
        assert_eq!(
            unsafe { region.oid(start.add(40)) }.unwrap_err(),
            CtlError::Bounds
        );
        assert_eq!(
            unsafe { region.oid(start.add(16)) }.unwrap_err(),
            CtlError::Bounds
        );
        let value = CRYPT_INTEGER_BLOB {
            cbData: 4,
            pbData: unsafe { start.add(12).cast_mut() },
        };
        assert_eq!(unsafe { region.blob(value, 4) }.unwrap(), b"1.2\0");
        let outside = CRYPT_INTEGER_BLOB {
            cbData: 4,
            pbData: unsafe { start.add(40).cast_mut() },
        };
        assert_eq!(
            unsafe { region.blob(outside, 4) }.unwrap_err(),
            CtlError::Bounds
        );
        let unterminated = [b'1'; 129];
        assert_eq!(
            unsafe { oid(unterminated.as_ptr()) }.unwrap_err(),
            CtlError::Bounds
        );
    }
    fn budget() -> Budget {
        Budget::until(unsafe { GetTickCount64() } + 10_000)
    }
    #[test]
    fn authenticated_synthetic_ctl_and_replay() {
        let result = verify(VALID, &[SIGNER], policy(), budget()).unwrap();
        assert_eq!(
            result.subject_identifiers,
            vec![(0..32).collect::<Vec<u8>>()]
        );
        assert_eq!(result.list_identifier, b"Aura research v1");
        assert_eq!(result.sequence, [7]);
        assert_eq!(result.signer_count, 1);
        let mut p = policy();
        p.previous = Some(Previous {
            sequence: &[7],
            encoded_sha256: result.encoded_sha256,
        });
        assert!(verify(VALID, &[SIGNER], p, budget()).is_ok());
        let mut p = policy();
        p.previous = Some(Previous {
            sequence: &[7],
            encoded_sha256: [0; 32],
        });
        assert_eq!(
            verify(VALID, &[SIGNER], p, budget()).unwrap_err(),
            CtlError::Rollback
        );
    }
    #[test]
    fn signed_negative_matrix() {
        for (bytes, expected) in [
            (
                include_bytes!(
                    "../../../tools/envbox-authroot-fixture/fixtures/missing-next-update.stl"
                )
                .as_slice(),
                CtlError::Time,
            ),
            (
                include_bytes!(
                    "../../../tools/envbox-authroot-fixture/fixtures/unknown-algorithm.stl"
                )
                .as_slice(),
                CtlError::Algorithm,
            ),
            (
                include_bytes!(
                    "../../../tools/envbox-authroot-fixture/fixtures/weak-signature.stl"
                )
                .as_slice(),
                CtlError::Algorithm,
            ),
            (
                include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/unsigned.stl")
                    .as_slice(),
                CtlError::Signature,
            ),
            (
                include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/tampered.stl")
                    .as_slice(),
                CtlError::Signature,
            ),
            (
                include_bytes!(
                    "../../../tools/envbox-authroot-fixture/fixtures/multiple-unknown.stl"
                )
                .as_slice(),
                CtlError::Signature,
            ),
            (
                include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/expired.stl")
                    .as_slice(),
                CtlError::Time,
            ),
            (
                include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/future.stl")
                    .as_slice(),
                CtlError::Time,
            ),
            (
                include_bytes!(
                    "../../../tools/envbox-authroot-fixture/fixtures/unknown-policy.stl"
                )
                .as_slice(),
                CtlError::Policy,
            ),
            (
                include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/wrong-list.stl")
                    .as_slice(),
                CtlError::Policy,
            ),
        ] {
            assert_eq!(
                verify(bytes, &[SIGNER], policy(), budget()).unwrap_err(),
                expected
            );
        }
        let wrong =
            include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/wrong-signer.der")
                .as_slice();
        assert_eq!(
            verify(VALID, &[wrong], policy(), budget()).unwrap_err(),
            CtlError::Signature
        );
        let wrong_eku =
            include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/wrong-eku.der")
                .as_slice();
        assert_eq!(
            verify(
                include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/wrong-eku.stl"),
                &[wrong_eku],
                policy(),
                budget()
            )
            .unwrap_err(),
            CtlError::Signer
        );
        let mut p = policy();
        p.previous = Some(Previous {
            sequence: &[7],
            encoded_sha256: [0; 32],
        });
        assert_eq!(
            verify(
                include_bytes!("../../../tools/envbox-authroot-fixture/fixtures/rollback.stl"),
                &[SIGNER],
                p,
                budget()
            )
            .unwrap_err(),
            CtlError::Rollback
        );
        assert_eq!(
            verify(
                include_bytes!(
                    "../../../tools/envbox-authroot-fixture/fixtures/multiple-known.stl"
                ),
                &[SIGNER],
                policy(),
                budget()
            )
            .unwrap()
            .signer_count,
            2
        );
    }
    #[test]
    fn cancellation_deadline_and_numeric_sequence() {
        assert_eq!(compare(&[0, 1], &[255]), Ordering::Greater);
        assert_eq!(compare(&[7, 0], &[7]), Ordering::Equal);
        assert_eq!(
            verify(VALID, &[SIGNER], policy(), Budget::until(0)).unwrap_err(),
            CtlError::Budget(Error::Deadline)
        );
        unsafe extern "C" fn cancel(_: *mut std::ffi::c_void) -> i32 {
            1
        }
        let b = unsafe { Budget::from_callback(u64::MAX, Some(cancel), ptr::null_mut()) };
        assert_eq!(
            verify(VALID, &[SIGNER], policy(), b).unwrap_err(),
            CtlError::Budget(Error::Cancelled)
        );
        unsafe extern "C" fn stop_during_decode(context: *mut std::ffi::c_void) -> i32 {
            let calls = unsafe { &*context.cast::<std::sync::atomic::AtomicUsize>() };
            i32::from(calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4)
        }
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let b = unsafe {
            Budget::from_callback(
                u64::MAX,
                Some(stop_during_decode),
                (&calls as *const std::sync::atomic::AtomicUsize)
                    .cast_mut()
                    .cast(),
            )
        };
        assert_eq!(
            verify(VALID, &[SIGNER], policy(), b).unwrap_err(),
            CtlError::Budget(Error::Cancelled)
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 5);
    }
    #[test]
    fn signer_time_and_bounded_inputs() {
        let mut p = policy();
        p.now_filetime = 0;
        assert_eq!(
            verify(VALID, &[SIGNER], p, budget()).unwrap_err(),
            CtlError::Time
        );
        let mut p = policy();
        p.now_filetime = u64::MAX;
        assert_eq!(
            verify(VALID, &[SIGNER], p, budget()).unwrap_err(),
            CtlError::Time
        );
        assert_eq!(
            verify(VALID, &[], policy(), budget()).unwrap_err(),
            CtlError::Bounds
        );
        assert_eq!(
            verify(&[], &[SIGNER], policy(), budget()).unwrap_err(),
            CtlError::Bounds
        );
        assert_eq!(
            verify(b"not ASN.1", &[SIGNER], policy(), budget()).unwrap_err(),
            CtlError::Decode
        );
        let mut p = policy();
        p.previous = Some(Previous {
            sequence: &[0; 17],
            encoded_sha256: [0; 32],
        });
        assert_eq!(
            verify(VALID, &[SIGNER], p, budget()).unwrap_err(),
            CtlError::Bounds
        );
        let mut p = policy();
        p.usage = b"unknown usage";
        assert_eq!(
            verify(VALID, &[SIGNER], p, budget()).unwrap_err(),
            CtlError::Bounds
        );
    }
}
