//! Offline local trust snapshot and explicit distrust; no chain or wire retrieval.
use crate::verification_scope::VerificationBudget;
use crate::{Budget, Error};
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        WebPkiServerVerifier,
    },
    pki_types::{CertificateDer, CertificateRevocationListDer, ServerName, UnixTime},
    DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{ptr, sync::Arc};
use windows_sys::Win32::{
    Foundation::{
        GetLastError, SetLastError, CRYPT_E_NOT_FOUND, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
        ERROR_SUCCESS, FILETIME,
    },
    Security::Cryptography::*,
    System::{
        Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY},
        SystemInformation::GetSystemTimeAsFileTime,
    },
};

const MAX_ITEMS: usize = 4096;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROPERTY: usize = 64 * 1024;
const MAX_OIDS: usize = 128;

/// Independent of DNS routing/fallback. Both modes verify TLS identity and chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum RevocationPolicy {
    Standard = 0,
    #[default]
    StrictOffline = 1,
}
impl TryFrom<u32> for RevocationPolicy {
    type Error = Error;
    fn try_from(value: u32) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Standard),
            1 => Ok(Self::StrictOffline),
            _ => Err(Error::Argument),
        }
    }
}

#[derive(Debug, Default)]
pub struct Snapshot {
    revocation_policy: RevocationPolicy,
    roots: Vec<CertificateDer<'static>>,
    intermediates: Vec<CertificateDer<'static>>,
    crls: Vec<CertificateRevocationListDer<'static>>,
    deny_sha1: Vec<[u8; 20]>,
    deny_sha256: Vec<[u8; 32]>,
    deny_signature_hash: Vec<Vec<u8>>,
    restricted_sha256: Vec<[u8; 32]>,
    // Native snapshots may consult already cached CDP material. Fixture
    // snapshots remain self-contained and never consult the host URL cache.
    use_url_cache: bool,
    #[cfg(any(test, feature = "fixture-trust"))]
    fixture_cached_crls: Option<Vec<Vec<u8>>>,
}

struct Store(HCERTSTORE);
impl Drop for Store {
    fn drop(&mut self) {
        unsafe {
            CertCloseStore(self.0, 0);
        }
    }
}
struct Cert(*const CERT_CONTEXT);
impl Drop for Cert {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CertFreeCertificateContext(self.0);
            }
        }
    }
}
struct Crl(*const CRL_CONTEXT);
impl Drop for Crl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CertFreeCRLContext(self.0);
            }
        }
    }
}
struct Ctl(*const CTL_CONTEXT);
impl Drop for Ctl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CertFreeCTLContext(self.0);
            }
        }
    }
}

#[derive(Default)]
struct Limits {
    items: usize,
    bytes: usize,
}
impl Limits {
    fn add(&mut self, size: usize, budget: Budget) -> Result<(), Error> {
        budget.check()?;
        self.items = self.items.checked_add(1).ok_or(Error::TrustSnapshot)?;
        self.bytes = self.bytes.checked_add(size).ok_or(Error::TrustSnapshot)?;
        if self.items > MAX_ITEMS || self.bytes > MAX_BYTES {
            return Err(Error::TrustSnapshot);
        }
        Ok(())
    }
}

// CryptoAPI owns these terminated OIDs. Limit policy parsing independently
// of the encoded context size, and never perform an unbounded C-string scan.
unsafe fn oid_bytes(oid: *const u8) -> Result<Vec<u8>, Error> {
    if oid.is_null() {
        return Err(Error::TrustSnapshot);
    }
    let mut result = Vec::new();
    for index in 0..128 {
        let byte = *oid.add(index);
        if byte == 0 {
            return if result.is_empty() {
                Err(Error::TrustSnapshot)
            } else {
                Ok(result)
            };
        }
        if !byte.is_ascii_digit() && byte != b'.' {
            return Err(Error::TrustSnapshot);
        }
        result.push(byte);
    }
    Err(Error::TrustSnapshot)
}

// Effective EKU includes the certificate property as well as DER extensions.
// Windows differentiates unrestricted usage from explicitly empty usage via
// GetLastError even when CertGetEnhancedKeyUsage succeeds with zero OIDs.
unsafe fn eligible(
    context: *const CERT_CONTEXT,
    limits: &mut Limits,
    budget: Budget,
) -> Result<bool, Error> {
    budget.check()?;
    if context.is_null() || (*context).pCertInfo.is_null() {
        return Err(Error::TrustSnapshot);
    }
    if CertVerifyTimeValidity(ptr::null(), (*context).pCertInfo) != 0 {
        return Ok(false);
    }
    let mut size = 0;
    SetLastError(0);
    if CertGetEnhancedKeyUsage(context, 0, ptr::null_mut(), &mut size) == 0
        || size as usize > MAX_PROPERTY
        || size < std::mem::size_of::<CTL_USAGE>() as u32
    {
        return Err(Error::TrustSnapshot);
    }
    limits.add(size as usize, budget)?;
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    let allocated = size;
    SetLastError(0);
    if CertGetEnhancedKeyUsage(context, 0, buffer.as_mut_ptr().cast(), &mut size) == 0 {
        return Err(Error::TrustSnapshot);
    }
    let last_error = GetLastError();
    budget.check()?;
    if size < std::mem::size_of::<CTL_USAGE>() as u32 || size > allocated {
        return Err(Error::TrustSnapshot);
    }
    let usage = &*buffer.as_ptr().cast::<CTL_USAGE>();
    if usage.cUsageIdentifier == 0 {
        return match last_error {
            value if value == CRYPT_E_NOT_FOUND as u32 => Ok(true),
            0 => Ok(false),
            _ => Err(Error::TrustSnapshot),
        };
    }
    if usage.cUsageIdentifier as usize > MAX_OIDS || usage.rgpszUsageIdentifier.is_null() {
        return Err(Error::TrustSnapshot);
    }
    let mut server_auth = false;
    for i in 0..usage.cUsageIdentifier as usize {
        budget.check()?;
        server_auth |= oid_bytes(*usage.rgpszUsageIdentifier.add(i))? == b"1.3.6.1.5.5.7.3.1";
    }
    Ok(server_auth)
}

// Windows' szOID_DISALLOWED_HASH refers to this property, not a DER digest.
// Use a detached context so lazily cached properties never write a host store.
fn signature_hash(der: &[u8]) -> Result<Vec<u8>, Error> {
    if der.is_empty() || der.len() > MAX_BYTES {
        return Err(Error::TrustSnapshot);
    }
    let certificate = Cert(unsafe {
        CertCreateCertificateContext(X509_ASN_ENCODING, der.as_ptr(), der.len() as u32)
    });
    if certificate.0.is_null() {
        return Err(Error::TrustSnapshot);
    }
    let mut size = 0;
    if unsafe {
        CertGetCertificateContextProperty(
            certificate.0,
            CERT_SIGNATURE_HASH_PROP_ID,
            ptr::null_mut(),
            &mut size,
        )
    } == 0
        || size == 0
        || size > 64
    {
        return Err(Error::TrustSnapshot);
    }
    let mut hash = vec![0; size as usize];
    let capacity = size;
    if unsafe {
        CertGetCertificateContextProperty(
            certificate.0,
            CERT_SIGNATURE_HASH_PROP_ID,
            hash.as_mut_ptr().cast(),
            &mut size,
        )
    } == 0
        || size == 0
        || size > capacity
    {
        return Err(Error::TrustSnapshot);
    }
    hash.truncate(size as usize);
    Ok(hash)
}

impl Snapshot {
    pub fn load(budget: Budget) -> Result<Self, Error> {
        Self::load_with_policy(budget, RevocationPolicy::Standard)
    }

    pub fn load_with_policy(budget: Budget, policy: RevocationPolicy) -> Result<Self, Error> {
        let mut snapshot = Self {
            revocation_policy: policy,
            ..Self::default()
        };
        let mut limits = Limits::default();
        for location in [
            CERT_SYSTEM_STORE_CURRENT_USER,
            CERT_SYSTEM_STORE_LOCAL_MACHINE,
        ] {
            for label in ["ROOT", "CA", "Disallowed"] {
                budget.check()?;
                let name: Vec<u16> = label.encode_utf16().chain([0]).collect();
                let handle = unsafe {
                    CertOpenStore(
                        CERT_STORE_PROV_SYSTEM_REGISTRY_W,
                        0,
                        0,
                        location | CERT_STORE_READONLY_FLAG | CERT_STORE_OPEN_EXISTING_FLAG,
                        name.as_ptr().cast(),
                    )
                };
                if handle.is_null() {
                    if unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND {
                        continue;
                    }
                    return Err(Error::TrustSnapshot);
                }
                let store = Store(handle);
                #[cfg(test)]
                eprintln!("trust snapshot: location={location:#x} store={label} certificates");
                snapshot.certificates(&store, label, &mut limits, budget)?;
                #[cfg(test)]
                eprintln!("trust snapshot: location={location:#x} store={label} crls");
                snapshot.crls(&store, &mut limits, budget)?;
                if label == "Disallowed" {
                    snapshot.ctls(&store, &mut limits, budget)?;
                }
            }
        }
        snapshot.cached_disallowed_ctl(&mut limits, budget)?;
        // Fixed-version Mozilla roots are explicit application trust inputs.
        // Full DER preserves local certificate/signature hash deny checks;
        // no AuthRoot promotion or online root provider is involved.
        for der in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
            limits.add(der.as_ref().len(), budget)?;
            let certificate = Cert(unsafe {
                CertCreateCertificateContext(
                    X509_ASN_ENCODING,
                    der.as_ref().as_ptr(),
                    der.as_ref().len() as u32,
                )
            });
            if certificate.0.is_null() {
                return Err(Error::TrustSnapshot);
            }
            if unsafe { eligible(certificate.0, &mut limits, budget)? } {
                snapshot.roots.push(der.clone());
            }
        }
        snapshot.use_url_cache = policy == RevocationPolicy::StrictOffline;
        budget.check()?;
        Ok(snapshot)
    }

    fn denied(&self, der: &[u8]) -> bool {
        self.deny_sha1.contains(&Sha1::digest(der).into())
            || self.deny_sha256.contains(&Sha256::digest(der).into())
    }
    fn signature_denied(&self, der: &[u8]) -> Result<bool, Error> {
        if self.deny_signature_hash.is_empty() {
            return Ok(false);
        }
        Ok(self.deny_signature_hash.contains(&signature_hash(der)?))
    }
    fn deny_certificate(&mut self, der: &[u8]) {
        self.deny_sha256.push(Sha256::digest(der).into());
    }
    fn restricted(&self, der: &[u8]) -> bool {
        self.restricted_sha256.contains(&Sha256::digest(der).into())
    }

    fn verify_peer_policy(
        &self,
        end: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
    ) -> Result<(), rustls::Error> {
        if intermediates.len() > 32
            || intermediates
                .iter()
                .try_fold(end.len(), |sum, cert| sum.checked_add(cert.len()))
                .is_none_or(|size| size > 1024 * 1024)
        {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            ));
        }
        if self.denied(end) || intermediates.iter().any(|cert| self.denied(cert)) {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        for certificate in std::iter::once(end).chain(intermediates) {
            match self.signature_denied(certificate) {
                Ok(true) => {
                    return Err(rustls::Error::InvalidCertificate(
                        rustls::CertificateError::ApplicationVerificationFailure,
                    ))
                }
                Ok(false) => {}
                Err(_) => {
                    return Err(rustls::Error::InvalidCertificate(
                        rustls::CertificateError::BadEncoding,
                    ))
                }
            }
        }
        if self.restricted(end) || intermediates.iter().any(|cert| self.restricted(cert)) {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::InvalidPurpose,
            ));
        }
        Ok(())
    }

    fn certificates(
        &mut self,
        store: &Store,
        label: &str,
        limits: &mut Limits,
        budget: Budget,
    ) -> Result<(), Error> {
        let mut current = Cert(ptr::null());
        loop {
            budget.check()?;
            current.0 = unsafe { CertEnumCertificatesInStore(store.0, current.0) };
            if current.0.is_null() {
                return if unsafe { GetLastError() } == CRYPT_E_NOT_FOUND as u32 {
                    Ok(())
                } else {
                    Err(Error::TrustSnapshot)
                };
            }
            let context = unsafe { &*current.0 };
            limits.add(context.cbCertEncoded as usize, budget)?;
            if context.cbCertEncoded == 0 || context.pbCertEncoded.is_null() {
                return Err(Error::TrustSnapshot);
            }
            let der = unsafe {
                std::slice::from_raw_parts(context.pbCertEncoded, context.cbCertEncoded as usize)
            }
            .to_vec();
            if label == "Disallowed" {
                self.deny_certificate(&der);
            } else if !unsafe { eligible(current.0, limits, budget)? } {
                self.restricted_sha256.push(Sha256::digest(&der).into());
            } else if label == "ROOT" {
                self.roots.push(der.into());
            } else {
                self.intermediates.push(der.into());
            }
        }
    }
    fn crls(&mut self, store: &Store, limits: &mut Limits, budget: Budget) -> Result<(), Error> {
        let mut current = Crl(ptr::null());
        loop {
            budget.check()?;
            current.0 = unsafe { CertEnumCRLsInStore(store.0, current.0) };
            if current.0.is_null() {
                return if unsafe { GetLastError() } == CRYPT_E_NOT_FOUND as u32 {
                    Ok(())
                } else {
                    Err(Error::TrustSnapshot)
                };
            }
            let context = unsafe { &*current.0 };
            limits.add(context.cbCrlEncoded as usize, budget)?;
            if context.cbCrlEncoded == 0 || context.pbCrlEncoded.is_null() {
                return Err(Error::TrustSnapshot);
            }
            self.crls.push(
                unsafe {
                    std::slice::from_raw_parts(context.pbCrlEncoded, context.cbCrlEncoded as usize)
                }
                .to_vec()
                .into(),
            );
        }
    }
    fn ctls(&mut self, store: &Store, limits: &mut Limits, budget: Budget) -> Result<(), Error> {
        let mut current = Ctl(ptr::null());
        loop {
            budget.check()?;
            current.0 = unsafe { CertEnumCTLsInStore(store.0, current.0) };
            if current.0.is_null() {
                return if unsafe { GetLastError() } == CRYPT_E_NOT_FOUND as u32 {
                    Ok(())
                } else {
                    Err(Error::TrustSnapshot)
                };
            }
            self.import_ctl(unsafe { &*current.0 }, limits, budget)?;
        }
    }

    fn import_ctl(
        &mut self,
        context: &CTL_CONTEXT,
        limits: &mut Limits,
        budget: Budget,
    ) -> Result<(), Error> {
        limits.add(context.cbCtlEncoded as usize, budget)?;
        if context.cbCtlEncoded == 0 || context.pbCtlEncoded.is_null() || context.pCtlInfo.is_null()
        {
            return Err(Error::TrustSnapshot);
        }
        let info = unsafe { &*context.pCtlInfo };
        #[cfg(test)]
        {
            let usage = if info.SubjectUsage.cUsageIdentifier == 1
                && !info.SubjectUsage.rgpszUsageIdentifier.is_null()
            {
                unsafe { oid_bytes(*info.SubjectUsage.rgpszUsageIdentifier) }.ok()
            } else {
                None
            };
            let algorithm = unsafe { oid_bytes(info.SubjectAlgorithm.pszObjId) }.ok();
            eprintln!("CTL policy: usage={usage:?} algorithm={algorithm:?} parameters_bytes={} this={}:{} next={}:{}", info.SubjectAlgorithm.Parameters.cbData, info.ThisUpdate.dwHighDateTime, info.ThisUpdate.dwLowDateTime, info.NextUpdate.dwHighDateTime, info.NextUpdate.dwLowDateTime);
            if info.cCTLEntry > 0 && !info.rgCTLEntry.is_null() {
                eprintln!(
                    "CTL entry widths/attributes: {:?}",
                    unsafe {
                        std::slice::from_raw_parts(
                            info.rgCTLEntry,
                            (info.cCTLEntry as usize).min(32),
                        )
                    }
                    .iter()
                    .map(|entry| (entry.SubjectIdentifier.cbData, entry.cAttribute))
                    .collect::<Vec<_>>()
                );
            }
        }
        #[cfg(test)]
        eprintln!(
            "CTL metadata: version={} usages={} entries={} extensions={} first_attributes={}",
            info.dwVersion,
            info.SubjectUsage.cUsageIdentifier,
            info.cCTLEntry,
            info.cExtension,
            if info.cCTLEntry > 0 && !info.rgCTLEntry.is_null() {
                unsafe { (*info.rgCTLEntry).cAttribute }
            } else {
                0
            }
        );
        if info.dwVersion != CTL_V1
            || info.cExtension != 0
            || info.SubjectUsage.cUsageIdentifier > 1
            || info.cCTLEntry as usize > MAX_ITEMS
            || (info.cCTLEntry > 0 && info.rgCTLEntry.is_null())
        {
            return Err(Error::TrustSnapshot);
        }
        if info.SubjectUsage.cUsageIdentifier == 1 {
            if info.SubjectUsage.rgpszUsageIdentifier.is_null()
                || unsafe { oid_bytes(*info.SubjectUsage.rgpszUsageIdentifier)? }
                    != b"1.3.6.1.4.1.311.10.3.30"
            {
                return Err(Error::TrustSnapshot);
            }
        }
        let parameters = &info.SubjectAlgorithm.Parameters;
        if parameters.cbData != 0
            && (parameters.cbData != 2
                || parameters.pbData.is_null()
                || unsafe { std::slice::from_raw_parts(parameters.pbData, 2) } != b"\x05\x00")
        {
            return Err(Error::TrustSnapshot);
        }
        let oid = unsafe { oid_bytes(info.SubjectAlgorithm.pszObjId)? };
        let mut now: FILETIME = unsafe { std::mem::zeroed() };
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        let stamp =
            |time: FILETIME| ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
        if stamp(info.ThisUpdate) == 0
            || stamp(info.ThisUpdate) > stamp(now)
            || (stamp(info.NextUpdate) != 0 && stamp(info.NextUpdate) <= stamp(now))
        {
            return Err(Error::TrustSnapshot);
        }
        let width = match oid.as_slice() {
            b"1.3.14.3.2.26" => 20,
            b"2.16.840.1.101.3.4.2.1" => 32,
            b"1.3.6.1.4.1.311.10.11.15" => 0,
            _ => return Err(Error::TrustSnapshot),
        };
        for index in 0..info.cCTLEntry as usize {
            let entry = unsafe { &*info.rgCTLEntry.add(index) };
            // Attribute/extension-specific Windows policy is not emulated.
            // Unknown forms reject the snapshot rather than lose distrust.
            if entry.cAttribute != 0 {
                return Err(Error::TrustSnapshot);
            }
            let identifier = &entry.SubjectIdentifier;
            let actual_width = identifier.cbData as usize;
            if (width != 0 && actual_width != width)
                || (width == 0 && (actual_width == 0 || actual_width > 64))
                || identifier.pbData.is_null()
            {
                return Err(Error::TrustSnapshot);
            }
            limits.add(actual_width, budget)?;
            let hash = unsafe { std::slice::from_raw_parts(identifier.pbData, actual_width) };
            if width == 0 {
                self.deny_signature_hash.push(hash.to_vec());
            } else if width == 20 {
                self.deny_sha1
                    .push(hash.try_into().map_err(|_| Error::TrustSnapshot)?);
            } else {
                self.deny_sha256
                    .push(hash.try_into().map_err(|_| Error::TrustSnapshot)?);
            }
        }
        Ok(())
    }

    fn cached_disallowed_ctl(&mut self, limits: &mut Limits, budget: Budget) -> Result<(), Error> {
        budget.check()?;
        let key: Vec<u16> = "SOFTWARE\\Microsoft\\SystemCertificates\\AuthRoot\\AutoUpdate"
            .encode_utf16()
            .chain([0])
            .collect();
        let name: Vec<u16> = "DisallowedCertEncodedCtl"
            .encode_utf16()
            .chain([0])
            .collect();
        let mut size = 0;
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_BINARY,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut size,
            )
        };
        if matches!(result, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            return Ok(());
        }
        if result != ERROR_SUCCESS || size == 0 || size as usize > MAX_BYTES {
            return Err(Error::TrustSnapshot);
        }
        limits.add(size as usize, budget)?;
        let mut encoded = vec![0u8; size as usize];
        let capacity = size;
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_BINARY,
                ptr::null_mut(),
                encoded.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if result != ERROR_SUCCESS || size == 0 || size > capacity {
            return Err(Error::TrustSnapshot);
        }
        budget.check()?;
        let context = Ctl(unsafe {
            CertCreateCTLContext(
                X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                encoded.as_ptr(),
                size,
            )
        });
        if context.0.is_null() {
            return Err(Error::TrustSnapshot);
        }
        self.import_ctl(unsafe { &*context.0 }, limits, budget)
    }

    pub(crate) fn verifier(mut self, budget: Budget) -> Result<Arc<dyn ServerCertVerifier>, Error> {
        budget.check()?;
        // rustls disables revocation with an empty list. Fixture snapshots must
        // reject it here; native verification defers acceptance until peer CDPs
        // supply usable cached CRLs, never accepting an unchecked result.
        if self.revocation_policy == RevocationPolicy::StrictOffline
            && self.crls.is_empty()
            && !self.use_url_cache
        {
            return Err(Error::RevocationUnknown);
        }
        let mut roots = RootCertStore::empty();
        for certificate in &self.roots {
            budget.check()?;
            if !self.denied(certificate)
                && !self.signature_denied(certificate)?
                && !self.restricted(certificate)
            {
                roots
                    .add(certificate.clone())
                    .map_err(|_| Error::TrustSnapshot)?;
            }
        }
        let mut candidates = Vec::new();
        for cert in &self.intermediates {
            budget.check()?;
            if !self.denied(cert) && !self.signature_denied(cert)? && !self.restricted(cert) {
                candidates.push(cert.clone());
            }
        }
        self.intermediates = candidates;
        let roots = Arc::new(roots);
        let verifier = standard_verifier(roots.clone(), self.crls.clone(), self.revocation_policy)?;
        budget.check()?;
        let cache_budget = self
            .use_url_cache
            .then(|| VerificationBudget::capture(budget))
            .transpose()?;
        Ok(Arc::new(OfflineVerifier {
            inner: verifier,
            snapshot: self,
            roots,
            cache_budget,
        }))
    }

    #[cfg(any(test, feature = "fixture-trust"))]
    pub fn fixture(
        roots: Vec<Vec<u8>>,
        intermediates: Vec<Vec<u8>>,
        crls: Vec<Vec<u8>>,
        denied: Vec<Vec<u8>>,
    ) -> Result<Self, Error> {
        let mut snapshot = Self::default();
        let mut fixture_limits = Limits::default();
        let budget = Budget {
            deadline: u64::MAX,
            cancelled: None,
            context: ptr::null_mut(),
        };
        for der in roots
            .iter()
            .chain(&intermediates)
            .chain(&crls)
            .chain(&denied)
        {
            fixture_limits.add(der.len(), budget)?;
        }
        let mut limits = Limits::default();
        for (list, root) in [(roots, true), (intermediates, false)] {
            for der in list {
                let certificate = Cert(unsafe {
                    CertCreateCertificateContext(X509_ASN_ENCODING, der.as_ptr(), der.len() as u32)
                });
                if certificate.0.is_null() {
                    return Err(Error::TrustSnapshot);
                }
                if !unsafe { eligible(certificate.0, &mut limits, budget)? } {
                    snapshot.restricted_sha256.push(Sha256::digest(&der).into());
                } else if root {
                    snapshot.roots.push(der.into());
                } else {
                    snapshot.intermediates.push(der.into());
                }
            }
        }
        snapshot.crls = crls.into_iter().map(Into::into).collect();
        for der in denied {
            snapshot.deny_certificate(&der);
        }
        Ok(snapshot)
    }

    /// Supply cache candidates to the standalone fixture without reading or
    /// writing Windows caches. Absent from product builds and the C ABI.
    #[cfg(any(test, feature = "fixture-trust"))]
    pub fn with_fixture_cached_crls(mut self, crls: Vec<Vec<u8>>) -> Result<Self, Error> {
        let mut limits = Limits::default();
        for der in &crls {
            limits.add(der.len(), Budget::until(u64::MAX))?;
        }
        self.fixture_cached_crls = Some(crls);
        self.use_url_cache = true;
        Ok(self)
    }

    /// Use real cache-only CDP reads with controlled fixture roots. No material
    /// is installed in Windows; this entry is absent from product builds.
    #[cfg(any(test, feature = "fixture-trust"))]
    pub fn with_fixture_native_cache(mut self) -> Self {
        self.fixture_cached_crls = None;
        self.use_url_cache = true;
        self
    }

    /// Select the independent TLS policy for controlled fixture trust only.
    #[cfg(any(test, feature = "fixture-trust"))]
    pub fn with_fixture_revocation_policy(mut self, policy: RevocationPolicy) -> Self {
        self.revocation_policy = policy;
        if policy == RevocationPolicy::Standard {
            self.use_url_cache = false;
        }
        self
    }
}

#[derive(Debug)]
struct OfflineVerifier {
    inner: Arc<WebPkiServerVerifier>,
    snapshot: Snapshot,
    roots: Arc<RootCertStore>,
    cache_budget: Option<VerificationBudget>,
}

fn standard_verifier(
    roots: Arc<RootCertStore>,
    crls: Vec<CertificateRevocationListDer<'static>>,
    policy: RevocationPolicy,
) -> Result<Arc<WebPkiServerVerifier>, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = WebPkiServerVerifier::builder_with_provider(roots, provider).with_crls(crls);
    let builder = match policy {
        // Standard PKI verification: missing revocation material is not a
        // prerequisite. Available CRLs still reject known revoked certificates.
        // No URL-cache reads or online retrieval occur in this mode.
        RevocationPolicy::Standard => builder.allow_unknown_revocation_status(),
        RevocationPolicy::StrictOffline => builder.enforce_revocation_expiration(),
    };
    builder.build().map_err(|_| Error::TrustSnapshot)
}

fn missing_revocation_material(error: &rustls::Error) -> bool {
    matches!(
        error,
        rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownRevocationStatus
                | rustls::CertificateError::ExpiredRevocationList
                | rustls::CertificateError::ExpiredRevocationListContext { .. }
        )
    )
}
impl ServerCertVerifier for OfflineVerifier {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.snapshot.verify_peer_policy(end, intermediates)?;
        let mut candidates: Vec<_> = intermediates.to_vec();
        candidates.extend(self.snapshot.intermediates.iter().cloned());
        // An empty CRL list disables revocation in rustls. Never accept that
        // result: native snapshots must first obtain usable cached material.
        let result = if self.snapshot.revocation_policy == RevocationPolicy::StrictOffline
            && self.snapshot.crls.is_empty()
        {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownRevocationStatus,
            ))
        } else {
            self.inner
                .verify_server_cert(end, &candidates, name, ocsp, now)
        };
        let Some(cache_budget) = self.cache_budget else {
            return result;
        };
        let Err(error) = result else {
            return result;
        };
        if !missing_revocation_material(&error) {
            return Err(error);
        }
        // This reads existing HTTP(S) cache keys only. It cannot download a
        // missing CRL or change trust anchors. Synchronous CryptoAPI work is
        // checked for deadline/cancellation, but cannot be interrupted mid-call.
        #[cfg(any(test, feature = "fixture-trust"))]
        let supplied = self.snapshot.fixture_cached_crls.clone();
        #[cfg(not(any(test, feature = "fixture-trust")))]
        let supplied: Option<Vec<Vec<u8>>> = None;
        let cached = cache_budget
            .collect(|budget| {
                supplied.map_or_else(
                    || {
                        crate::offline_crl::for_certificates(
                            std::iter::once(end.as_ref())
                                .chain(candidates.iter().map(AsRef::as_ref)),
                            budget,
                        )
                    },
                    Ok,
                )
            })
            .map_err(|error| rustls::Error::General(format!("offline CRL cache: {error:?}")))?;
        if cached.is_empty() {
            return Err(error);
        }
        let mut crls: Vec<_> = cached.into_iter().map(Into::into).collect();
        crls.extend(self.snapshot.crls.iter().cloned());
        // Cached DER is only a candidate. The standard verifier must check
        // issuer/signature, freshness, revocation, chain, purpose and name.
        standard_verifier(self.roots.clone(), crls, self.snapshot.revocation_policy)
            .map_err(|error| rustls::Error::General(format!("offline CRL verifier: {error:?}")))?
            .verify_server_cert(end, &candidates, name, ocsp, now)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls12_signature(message, certificate, signed)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls13_signature(message, certificate, signed)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }
    fn root_hint_subjects(&self) -> Option<&[rustls::DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> Budget {
        Budget {
            deadline: u64::MAX,
            cancelled: None,
            context: ptr::null_mut(),
        }
    }

    // A locally encoded certificate context is sufficient for property/EKU
    // tests. Its dummy signature is never used as a TLS trust anchor.
    fn context_certificate(serial: u8) -> Vec<u8> {
        fn der(tag: u8, content: &[u8]) -> Vec<u8> {
            let mut result = vec![tag];
            if content.len() < 128 {
                result.push(content.len() as u8);
            } else if content.len() < 256 {
                result.extend([0x81, content.len() as u8]);
            } else {
                result.extend([0x82, (content.len() >> 8) as u8, content.len() as u8]);
            }
            result.extend(content);
            result
        }
        fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
            der(0x30, &parts.concat())
        }
        let algorithm = seq(&[
            der(6, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 1, 11]),
            der(5, &[]),
        ]);
        let name = seq(&[der(
            0x31,
            &seq(&[der(6, &[0x55, 4, 3]), der(0x0c, b"offline-test")]),
        )]);
        let validity = seq(&[der(0x17, b"200101000000Z"), der(0x17, b"491231235959Z")]);
        let key_algorithm = seq(&[
            der(6, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 1, 1]),
            der(5, &[]),
        ]);
        let key = seq(&[der(2, &[1, 0, 1]), der(2, &[1, 0, 1])]);
        let public_key = seq(&[key_algorithm, der(3, &[vec![0], key].concat())]);
        let tbs = seq(&[
            der(0xa0, &der(2, &[2])),
            der(2, &[serial]),
            algorithm.clone(),
            name.clone(),
            validity,
            name,
            public_key,
        ]);
        seq(&[tbs, algorithm, der(3, &[0, 1])])
    }

    #[test]
    fn detached_effective_eku_and_signature_distrust() {
        let der = context_certificate(1);
        let cert = Cert(unsafe {
            CertCreateCertificateContext(X509_ASN_ENCODING, der.as_ptr(), der.len() as u32)
        });
        assert!(!cert.0.is_null());
        assert!(unsafe { eligible(cert.0, &mut Limits::default(), budget()) }.unwrap());
        let mut usage: CTL_USAGE = unsafe { std::mem::zeroed() };
        assert_ne!(unsafe { CertSetEnhancedKeyUsage(cert.0, &usage) }, 0);
        assert!(!unsafe { eligible(cert.0, &mut Limits::default(), budget()) }.unwrap());
        let mut client_oid = b"1.3.6.1.5.5.7.3.2\0".to_vec();
        let mut client = client_oid.as_mut_ptr();
        usage.cUsageIdentifier = 1;
        usage.rgpszUsageIdentifier = &mut client;
        assert_ne!(unsafe { CertSetEnhancedKeyUsage(cert.0, &usage) }, 0);
        assert!(!unsafe { eligible(cert.0, &mut Limits::default(), budget()) }.unwrap());
        let mut server_oid = b"1.3.6.1.5.5.7.3.1\0".to_vec();
        let mut server = server_oid.as_mut_ptr();
        usage.rgpszUsageIdentifier = &mut server;
        assert_ne!(unsafe { CertSetEnhancedKeyUsage(cert.0, &usage) }, 0);
        assert!(unsafe { eligible(cert.0, &mut Limits::default(), budget()) }.unwrap());
        let hash = signature_hash(&der).unwrap();
        assert_eq!(hash.len(), 32);
        let mut snapshot = Snapshot::default();
        snapshot.deny_signature_hash.push(hash);
        let peer = CertificateDer::from(der);
        assert_eq!(
            Error::from(snapshot.verify_peer_policy(&peer, &[]).unwrap_err()),
            Error::Disallowed
        );
        assert_eq!(
            Error::from(
                snapshot
                    .verify_peer_policy(&CertificateDer::from(context_certificate(2)), &[peer])
                    .unwrap_err()
            ),
            Error::Disallowed
        );
        assert!(signature_hash(b"bad DER").is_err());
    }

    #[test]
    fn empty_crls_cannot_disable_revocation() {
        assert_eq!(
            Snapshot::default().verifier(budget()).unwrap_err(),
            Error::RevocationUnknown
        );
    }

    #[test]
    fn cache_retry_is_limited_to_missing_or_expired_revocation_material() {
        use rustls::CertificateError;
        for certificate_error in [
            CertificateError::UnknownRevocationStatus,
            CertificateError::ExpiredRevocationList,
        ] {
            assert!(missing_revocation_material(
                &rustls::Error::InvalidCertificate(certificate_error)
            ));
        }
        for certificate_error in [
            CertificateError::Revoked,
            CertificateError::UnknownIssuer,
            CertificateError::NotValidForName,
            CertificateError::Expired,
            CertificateError::BadSignature,
            CertificateError::InvalidPurpose,
        ] {
            assert!(!missing_revocation_material(
                &rustls::Error::InvalidCertificate(certificate_error)
            ));
        }
        assert!(!missing_revocation_material(&rustls::Error::General(
            "cache failure".to_owned()
        )));
    }

    #[test]
    fn native_empty_crls_and_cache_miss_cannot_accept_a_peer() {
        let mut snapshot = Snapshot::default();
        snapshot.use_url_cache = true;
        snapshot.roots.push(context_certificate(1).into());
        let verifier = snapshot.verifier(budget()).unwrap();
        let end = CertificateDer::from(context_certificate(2));
        let result = verifier.verify_server_cert(
            &end,
            &[],
            &ServerName::try_from("offline-test").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(matches!(
            result,
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownRevocationStatus
            ))
        ));
    }

    #[test]
    fn snapshot_limits_and_cancellation_fail_closed() {
        let mut limits = Limits::default();
        assert_eq!(
            limits.add(MAX_BYTES + 1, budget()),
            Err(Error::TrustSnapshot)
        );
        let mut limits = Limits {
            items: MAX_ITEMS,
            bytes: 0,
        };
        assert_eq!(limits.add(0, budget()), Err(Error::TrustSnapshot));
        unsafe extern "C" fn cancelled(_: *mut std::ffi::c_void) -> i32 {
            1
        }
        let mut cancelled_budget = budget();
        cancelled_budget.cancelled = Some(cancelled);
        assert_eq!(
            Snapshot::load(cancelled_budget).unwrap_err(),
            Error::Cancelled
        );
        assert_eq!(
            Snapshot::load(Budget {
                deadline: 0,
                ..budget()
            })
            .unwrap_err(),
            Error::Deadline
        );
    }

    #[test]
    fn hash_ctl_imports_supported_algorithms_and_rejects_unknown_policy() {
        for (oid, width) in [
            (b"1.3.14.3.2.26\0".as_slice(), 20usize),
            (b"2.16.840.1.101.3.4.2.1\0".as_slice(), 32),
        ] {
            let der = b"certificate bytes";
            let mut digest = if width == 20 {
                Sha1::digest(der).to_vec()
            } else {
                Sha256::digest(der).to_vec()
            };
            let mut entry: CTL_ENTRY = unsafe { std::mem::zeroed() };
            entry.SubjectIdentifier.cbData = width as u32;
            entry.SubjectIdentifier.pbData = digest.as_mut_ptr();
            let mut info: CTL_INFO = unsafe { std::mem::zeroed() };
            info.SubjectAlgorithm.pszObjId = oid.as_ptr().cast_mut();
            info.ThisUpdate.dwLowDateTime = 1;
            info.cCTLEntry = 1;
            info.rgCTLEntry = &mut entry;
            let encoded = [0u8];
            let mut context: CTL_CONTEXT = unsafe { std::mem::zeroed() };
            context.pCtlInfo = &mut info;
            context.cbCtlEncoded = 1;
            context.pbCtlEncoded = encoded.as_ptr().cast_mut();
            let mut snapshot = Snapshot::default();
            snapshot
                .import_ctl(&context, &mut Limits::default(), budget())
                .unwrap();
            assert!(snapshot.denied(der));
            entry.cAttribute = 1;
            assert_eq!(
                Snapshot::default().import_ctl(&context, &mut Limits::default(), budget()),
                Err(Error::TrustSnapshot)
            );
            entry.cAttribute = 0;
            entry.SubjectIdentifier.cbData -= 1;
            assert_eq!(
                Snapshot::default().import_ctl(&context, &mut Limits::default(), budget()),
                Err(Error::TrustSnapshot)
            );
            entry.SubjectIdentifier.cbData += 1;
            info.cExtension = 1;
            assert_eq!(
                Snapshot::default().import_ctl(&context, &mut Limits::default(), budget()),
                Err(Error::TrustSnapshot)
            );
            info.cExtension = 0;
            info.SubjectAlgorithm.pszObjId = b"1.2.3\0".as_ptr().cast_mut();
            assert_eq!(
                Snapshot::default().import_ctl(&context, &mut Limits::default(), budget()),
                Err(Error::TrustSnapshot)
            );
        }
    }

    #[test]
    fn certificate_size_failure_is_not_explicit_distrust() {
        let mut snapshot = Snapshot::default();
        let end = CertificateDer::from(b"end".as_slice());
        let ca = CertificateDer::from(b"ca".as_slice());
        assert_eq!(
            Error::from(
                snapshot
                    .verify_peer_policy(&end, &vec![ca.clone(); 33])
                    .unwrap_err()
            ),
            Error::Certificate
        );
        let oversized = CertificateDer::from(vec![0; 1024 * 1024 + 1]);
        assert_eq!(
            Error::from(snapshot.verify_peer_policy(&oversized, &[]).unwrap_err()),
            Error::Certificate
        );
        snapshot.deny_certificate(&ca);
        assert_eq!(
            Error::from(
                snapshot
                    .verify_peer_policy(&end, &[ca.clone()])
                    .unwrap_err()
            ),
            Error::Disallowed
        );
        assert_eq!(
            Error::from(snapshot.verify_peer_policy(&ca, &[]).unwrap_err()),
            Error::Disallowed
        );
        snapshot
            .restricted_sha256
            .push(Sha256::digest(end.as_ref()).into());
        assert_eq!(
            Error::from(snapshot.verify_peer_policy(&end, &[]).unwrap_err()),
            Error::Certificate
        );
    }

    #[test]
    #[ignore = "read-only host trust diagnostics; policy depends on this Windows installation"]
    fn readonly_native_snapshot_diagnostic() {
        let result = Snapshot::load(budget());
        match result {
            Ok(snapshot) => {
                eprintln!("readonly native snapshot: roots={} intermediates={} crls={} deny_sha1={} deny_sha256={} deny_signature_hash={} restricted={}",snapshot.roots.len(),snapshot.intermediates.len(),snapshot.crls.len(),snapshot.deny_sha1.len(),snapshot.deny_sha256.len(),snapshot.deny_signature_hash.len(),snapshot.restricted_sha256.len());
                match snapshot.verifier(budget()) {
                    Ok(_) => eprintln!("readonly native verifier: accepted"),
                    Err(error) => eprintln!("readonly native verifier: {error:?}"),
                }
            }
            Err(error) => eprintln!("readonly native snapshot: {error:?}"),
        }
    }
}
