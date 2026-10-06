//! Unauthenticated CRL candidates from the existing Windows URL cache only.
//!
//! No chain construction, network retrieval, cache writes, or certificate store
//! writes occur here. The caller must validate issuer, signature and freshness.
//! Synchronous CAPI calls cannot be interrupted mid-call: check the shared
//! deadline/cancellation before and after every call and supply a finite timeout.
//! The native verifier recovers the shared Budget from an active caller-thread
//! query scope; callback/context never move into the Send + Sync verifier.
use crate::{Budget, Error};
use std::{collections::HashSet, ffi::c_void, mem, ptr, slice};
use windows_sys::Win32::{
    Foundation::{GetLastError, CRYPT_E_NOT_FOUND, ERROR_FILE_NOT_FOUND},
    Security::Cryptography::*,
    System::SystemInformation::GetTickCount64,
};

const MAX_CERTIFICATES: usize = 64;
const MAX_URLS: usize = 32;
const MAX_URL_UNITS: usize = 4096;
const MAX_URL_BUFFER: usize = 512 * 1024;
const MAX_DER_BYTES: usize = 8 * 1024 * 1024;

struct Cert(*const CERT_CONTEXT);
impl Drop for Cert {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CertFreeCertificateContext(self.0) };
        }
    }
}
struct Crl(*const CRL_CONTEXT);
impl Drop for Crl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CertFreeCRLContext(self.0) };
        }
    }
}

fn cache_key(url: &str) -> Result<(), Error> {
    if url.is_empty()
        || url.encode_utf16().count() > MAX_URL_UNITS
        || url
            .bytes()
            .any(|byte| byte <= 0x20 || byte == 0x7f || byte == b'\\')
        || url.contains('#')
    {
        return Err(Error::TrustSnapshot);
    }
    let uri: http::Uri = url.parse().map_err(|_| Error::TrustSnapshot)?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri
            .authority()
            .is_none_or(|authority| authority.as_str().contains('@'))
        || uri.host().is_none_or(str::is_empty)
    {
        return Err(Error::TrustSnapshot);
    }
    Ok(())
}

fn contains_range(base: usize, length: usize, pointer: usize, size: usize) -> bool {
    pointer >= base
        && pointer
            .checked_add(size)
            .is_some_and(|end| base.checked_add(length).is_some_and(|limit| end <= limit))
}

fn distribution_points(cert: &Cert, budget: Budget) -> Result<Vec<String>, Error> {
    budget.check()?;
    let mut size = 0u32;
    let ok = unsafe {
        CryptGetObjectUrl(
            URL_OID_CERTIFICATE_CRL_DIST_POINT,
            cert.0.cast(),
            CRYPT_GET_URL_FROM_EXTENSION,
            ptr::null_mut(),
            &mut size,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    let error = if ok == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    budget.check()?;
    if ok == 0 {
        return if error == CRYPT_E_NOT_FOUND as u32 {
            Ok(Vec::new())
        } else {
            Err(Error::TrustSnapshot)
        };
    }
    let expected = size as usize;
    if !(mem::size_of::<CRYPT_URL_ARRAY>()..=MAX_URL_BUFFER).contains(&expected) {
        return Err(Error::TrustSnapshot);
    }
    // Pointer-aligned allocation: the API embeds pointers in this buffer.
    let mut storage = vec![0usize; expected.div_ceil(mem::size_of::<usize>())];
    let array = storage.as_mut_ptr().cast::<CRYPT_URL_ARRAY>();
    budget.check()?;
    let ok = unsafe {
        CryptGetObjectUrl(
            URL_OID_CERTIFICATE_CRL_DIST_POINT,
            cert.0.cast(),
            CRYPT_GET_URL_FROM_EXTENSION,
            array,
            &mut size,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    budget.check()?;
    // The API may use fewer bytes than its initial size estimate. Restrict all
    // embedded pointers to the actual returned region, not allocation padding.
    let actual = size as usize;
    if ok == 0 || actual < mem::size_of::<CRYPT_URL_ARRAY>() || actual > expected {
        return Err(Error::TrustSnapshot);
    }
    let base = storage.as_ptr() as usize;
    let header = unsafe { &*array };
    let count = header.cUrl as usize;
    if count > MAX_URLS
        || (count != 0
            && !contains_range(
                base,
                actual,
                header.rgwszUrl as usize,
                count * mem::size_of::<*mut u16>(),
            ))
    {
        return Err(Error::TrustSnapshot);
    }
    let mut urls = Vec::with_capacity(count);
    for index in 0..count {
        budget.check()?;
        // read_unaligned also rejects any need to trust API pointer alignment.
        let url = unsafe { ptr::read_unaligned(header.rgwszUrl.add(index)) };
        let mut units = Vec::new();
        for offset in 0..=MAX_URL_UNITS {
            let address = (url as usize)
                .checked_add(offset * 2)
                .ok_or(Error::TrustSnapshot)?;
            if !contains_range(base, actual, address, 2) {
                return Err(Error::TrustSnapshot);
            }
            let unit = unsafe { ptr::read_unaligned(address as *const u16) };
            if unit == 0 {
                break;
            }
            if offset == MAX_URL_UNITS {
                return Err(Error::TrustSnapshot);
            }
            units.push(unit);
        }
        let value = String::from_utf16(&units).map_err(|_| Error::TrustSnapshot)?;
        cache_key(&value)?;
        urls.push(value);
    }
    Ok(urls)
}

fn cached_crl(url: &str, budget: Budget) -> Result<Option<Vec<u8>>, Error> {
    cache_key(url)?;
    budget.check()?;
    let remaining = budget.deadline.saturating_sub(unsafe { GetTickCount64() });
    if remaining == 0 {
        return Err(Error::Deadline);
    }
    let timeout = remaining.min(u32::MAX as u64) as u32;
    let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let mut object: *mut c_void = ptr::null_mut();
    let ok = unsafe {
        CryptRetrieveObjectByUrlW(
            wide.as_ptr(),
            CONTEXT_OID_CRL,
            CRYPT_CACHE_ONLY_RETRIEVAL | CRYPT_DONT_CACHE_RESULT,
            timeout,
            &mut object,
            0,
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
        )
    };
    let error = if ok == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    // No MULTIPLE_OBJECTS flag: the result is a single CRL context, not a store.
    let crl = Crl(object.cast());
    budget.check()?;
    if ok == 0 {
        return if error == CRYPT_E_NOT_FOUND as u32 || error == ERROR_FILE_NOT_FOUND {
            Ok(None)
        } else {
            Err(Error::TrustSnapshot)
        };
    }
    if crl.0.is_null() {
        return Err(Error::TrustSnapshot);
    }
    let context = unsafe { &*crl.0 };
    if context.cbCrlEncoded == 0
        || context.cbCrlEncoded as usize > MAX_DER_BYTES
        || context.pbCrlEncoded.is_null()
    {
        return Err(Error::TrustSnapshot);
    }
    let der = unsafe { slice::from_raw_parts(context.pbCrlEncoded, context.cbCrlEncoded as usize) }
        .to_vec();
    budget.check()?;
    Ok(Some(der))
}

/// Return bounded, **unauthenticated** cached CRL candidates for certificate CDPs.
pub(crate) fn for_certificates<'a>(
    certificates: impl IntoIterator<Item = &'a [u8]>,
    budget: Budget,
) -> Result<Vec<Vec<u8>>, Error> {
    let mut urls = HashSet::new();
    let mut result = Vec::new();
    let mut certificate_bytes = 0usize;
    let mut crl_bytes = 0usize;
    let mut url_count = 0usize;
    for (index, der) in certificates.into_iter().enumerate() {
        budget.check()?;
        certificate_bytes = certificate_bytes
            .checked_add(der.len())
            .ok_or(Error::TrustSnapshot)?;
        if index >= MAX_CERTIFICATES
            || der.is_empty()
            || certificate_bytes
                .checked_add(crl_bytes)
                .is_none_or(|total| total > MAX_DER_BYTES)
        {
            return Err(Error::TrustSnapshot);
        }
        let cert = Cert(unsafe {
            CertCreateCertificateContext(X509_ASN_ENCODING, der.as_ptr(), der.len() as u32)
        });
        budget.check()?;
        if cert.0.is_null() {
            return Err(Error::TrustSnapshot);
        }
        for url in distribution_points(&cert, budget)? {
            url_count += 1;
            if url_count > MAX_URLS {
                return Err(Error::TrustSnapshot);
            }
            if !urls.insert(url.clone()) {
                continue;
            }
            if let Some(der) = cached_crl(&url, budget)? {
                crl_bytes = crl_bytes
                    .checked_add(der.len())
                    .ok_or(Error::TrustSnapshot)?;
                if crl_bytes
                    .checked_add(certificate_bytes)
                    .is_none_or(|total| total > MAX_DER_BYTES)
                {
                    return Err(Error::TrustSnapshot);
                }
                if !result.contains(&der) {
                    result.push(der);
                }
            }
        }
    }
    budget.check()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn budget() -> Budget {
        Budget::until(unsafe { GetTickCount64() } + 5000)
    }

    // Parseable detached certificate; its deliberately dummy signature is never
    // trusted. No private key, certificate-store entry or cache entry is created.
    fn certificate(url: Option<&str>) -> Vec<u8> {
        fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
            let mut value = vec![tag];
            if content.len() < 128 {
                value.push(content.len() as u8);
            } else {
                value.extend([0x82, (content.len() >> 8) as u8, content.len() as u8]);
            }
            value.extend(content);
            value
        }
        fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
            tlv(0x30, &parts.concat())
        }
        let algorithm = seq(&[tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 2])]);
        let mut public_key = vec![0, 4];
        public_key.extend([0u8; 64]);
        let mut parts = vec![
            tlv(0xa0, &tlv(2, &[2])),
            tlv(2, &[1]),
            algorithm.clone(),
            seq(&[]),
            seq(&[tlv(0x17, b"200101000000Z"), tlv(0x17, b"300101000000Z")]),
            seq(&[]),
            seq(&[
                seq(&[
                    tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 2, 1]),
                    tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 3, 1, 7]),
                ]),
                tlv(3, &public_key),
            ]),
        ];
        if let Some(url) = url {
            let points = seq(&[seq(&[tlv(0xa0, &tlv(0xa0, &tlv(0x86, url.as_bytes())))])]);
            parts.push(tlv(
                0xa3,
                &seq(&[seq(&[tlv(6, &[0x55, 0x1d, 0x1f]), tlv(4, &points)])]),
            ));
        }
        seq(&[seq(&parts), algorithm, tlv(3, &[0, 1])])
    }

    #[test]
    fn detached_certificate_cdp_and_certificate_limit() {
        let cert = certificate(Some("https://envbox-offline-crl.invalid/detached.crl"));
        assert_eq!(
            for_certificates([cert.as_slice()], budget()),
            Ok(Vec::new())
        );
        let no_cdp = certificate(None);
        assert_eq!(
            for_certificates([no_cdp.as_slice(); MAX_CERTIFICATES], budget()),
            Ok(Vec::new())
        );
        assert_eq!(
            for_certificates([no_cdp.as_slice(); MAX_CERTIFICATES + 1], budget()),
            Err(Error::TrustSnapshot)
        );
        let forbidden = certificate(Some("file:///must-not-open.crl"));
        assert_eq!(
            for_certificates([forbidden.as_slice()], budget()),
            Err(Error::TrustSnapshot)
        );
        assert_eq!(
            for_certificates([cert.as_slice(); MAX_URLS + 1], budget()),
            Err(Error::TrustSnapshot)
        );
    }

    #[test]
    fn only_unambiguous_http_cache_keys() {
        for url in [
            "https://crl.example.test/a.crl",
            "http://127.0.0.1:9/a",
            "https://[::1]/a?b=c",
        ] {
            assert_eq!(cache_key(url), Ok(()), "{url}");
        }
        for url in [
            "file:///tmp/a",
            "ldap://host/a",
            "ftp://host/a",
            "https://u:p@host/a",
            "https:///a",
            "https://host/a#b",
            "https://host\\a",
            "https://host/a\0",
            "https://host/a b",
            "//host/a",
        ] {
            assert_eq!(cache_key(url), Err(Error::TrustSnapshot), "{url}");
        }
        assert_eq!(
            cache_key(&format!("https://host/{}", "a".repeat(MAX_URL_UNITS))),
            Err(Error::TrustSnapshot)
        );
    }

    #[test]
    fn pointer_ranges_reject_overflow_and_outside_storage() {
        assert!(contains_range(100, 10, 100, 10));
        assert!(!contains_range(100, 10, 99, 1));
        assert!(!contains_range(100, 10, 110, 1));
        assert!(!contains_range(100, 10, usize::MAX, 2));
    }

    #[test]
    fn invalid_certificate_and_budget_fail_closed() {
        assert_eq!(
            for_certificates([b"invalid".as_slice()], budget()),
            Err(Error::TrustSnapshot)
        );
        assert_eq!(
            for_certificates(std::iter::empty(), Budget::until(0)),
            Err(Error::Deadline)
        );
        assert_eq!(
            for_certificates(std::iter::empty(), budget()),
            Ok(Vec::new())
        );
    }

    #[test]
    fn native_cache_miss_does_not_fetch() {
        // .invalid is reserved; the unique path has never been inserted by this test.
        let url = format!(
            "https://envbox-offline-crl.invalid/{}-{}.crl",
            std::process::id(),
            unsafe { GetTickCount64() }
        );
        assert_eq!(cached_crl(&url, budget()), Ok(None));
    }

    #[test]
    fn native_cache_miss_never_connects_to_owned_listener() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "http://127.0.0.1:{}/envbox-offline-{}-{}.crl",
            listener.local_addr().unwrap().port(),
            std::process::id(),
            unsafe { GetTickCount64() }
        );
        assert_eq!(cached_crl(&url, budget()), Ok(None));
        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            result => panic!("cache-only retrieval contacted the owned listener: {result:?}"),
        }
    }
}
