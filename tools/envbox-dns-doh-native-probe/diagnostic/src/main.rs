// Reuse the production source modules directly in a standalone diagnostic so
// the native verifier is identical without adding a public debug API or an
// accept-all certificate verifier to the product crate.
#[path = "../../../../crates/envbox-dns-doh/src/budget.rs"]
mod budget;
#[path = "../../../../crates/envbox-dns-doh/src/error.rs"]
mod error;
#[path = "../../../../crates/envbox-dns-doh/src/executor.rs"]
mod executor;
#[path = "../../../../crates/envbox-dns-doh/src/transport.rs"]
mod transport;
pub mod trust {
    include!(concat!(env!("OUT_DIR"), "/trust.rs"));

    // This helper is compiled only into the standalone diagnostic. It exposes
    // hashes for read-only comparison with the local Windows store without
    // changing the product crate's public API or trust policy.
    pub fn accepted_root_sha256(snapshot: &Snapshot) -> Vec<String> {
        use sha2::{Digest, Sha256};
        snapshot
            .roots
            .iter()
            .map(|der| {
                Sha256::digest(der.as_ref())
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect()
            })
            .collect()
    }

    // Locate the public Cloudflare root in the same physical ROOT stores and
    // run the production eligibility predicate. This is read-only and exists
    // only in the diagnostic binary, so it does not widen the product trust
    // set or change the production API.
    pub fn native_ssl_root_eligibility() -> Vec<String> {
        use sha2::Digest;
        use windows_sys::Win32::{
            Foundation::{GetLastError, SetLastError},
            Security::Cryptography::*,
        };
        const TARGET: [u8; 32] = [
            0x34, 0x17, 0xbb, 0x06, 0xcc, 0x60, 0x07, 0xda, 0x1b, 0x96, 0x1c, 0x92, 0x0b, 0x8a,
            0xb4, 0xce, 0x3f, 0xad, 0x82, 0x0e, 0x4a, 0xa3, 0x0b, 0x9a, 0xcb, 0xc4, 0xa7, 0x4e,
            0xbd, 0xce, 0xbc, 0x65,
        ];
        let mut result = Vec::new();
        for location in [
            CERT_SYSTEM_STORE_CURRENT_USER,
            CERT_SYSTEM_STORE_LOCAL_MACHINE,
        ] {
            for label in ["ROOT", "AuthRoot"] {
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
                    result.push(format!(
                        "location={location:#x} store={label} open_error={}",
                        unsafe { GetLastError() }
                    ));
                    continue;
                }
                let store = Store(handle);
                let mut current = Cert(std::ptr::null());
                let mut entries = 0usize;
                let mut matched = false;
                loop {
                    current.0 = unsafe { CertEnumCertificatesInStore(store.0, current.0) };
                    if current.0.is_null() {
                        break;
                    }
                    entries += 1;
                    let context = unsafe { &*current.0 };
                    if context.cbCertEncoded == 0 || context.pbCertEncoded.is_null() {
                        continue;
                    }
                    let der = unsafe {
                        std::slice::from_raw_parts(
                            context.pbCertEncoded,
                            context.cbCertEncoded as usize,
                        )
                    };
                    if sha2::Sha256::digest(der).as_slice() != TARGET {
                        continue;
                    }
                    matched = true;
                    let validity =
                        unsafe { CertVerifyTimeValidity(std::ptr::null(), context.pCertInfo) };
                    let mut size = 0u32;
                    unsafe { SetLastError(0) };
                    let first = unsafe {
                        CertGetEnhancedKeyUsage(current.0, 0, std::ptr::null_mut(), &mut size)
                    };
                    let first_error = unsafe { GetLastError() };
                    let mut second = 0i32;
                    let mut second_error = 0u32;
                    let mut usage_count = None;
                    if first != 0 && size >= std::mem::size_of::<CTL_USAGE>() as u32 {
                        let mut bytes = vec![0u8; size as usize];
                        let mut actual = size;
                        unsafe { SetLastError(0) };
                        second = unsafe {
                            CertGetEnhancedKeyUsage(
                                current.0,
                                0,
                                bytes.as_mut_ptr().cast(),
                                &mut actual,
                            )
                        };
                        second_error = unsafe { GetLastError() };
                        if second != 0 {
                            usage_count = Some(unsafe {
                                (&*bytes.as_ptr().cast::<CTL_USAGE>()).cUsageIdentifier
                            });
                        }
                    }
                    let accepted = unsafe {
                        eligible(
                            current.0,
                            &mut Limits::default(),
                            Budget::until(crate::deadline()),
                        )
                    };
                    let mut properties = Vec::new();
                    for property_id in [83u32, 84, 98, 105] {
                        let mut property_size = 0u32;
                        unsafe { SetLastError(0) };
                        let present = unsafe {
                            CertGetCertificateContextProperty(
                                current.0,
                                property_id,
                                std::ptr::null_mut(),
                                &mut property_size,
                            )
                        };
                        let property_error = unsafe { GetLastError() };
                        properties.push(format!(
                        "{property_id}:present={present},size={property_size},error={property_error}"
                    ));
                    }
                    result.push(format!(
                    "location={location:#x} store={label} entries={entries} sha256={:02X?} validity={} eku_first={} eku_first_last_error={} eku_second={} eku_second_last_error={} eku_count={:?} properties=[{}] eligible={accepted:?}",
                    TARGET,
                    validity,
                    first,
                    first_error,
                    second,
                    second_error,
                    usage_count,
                    properties.join(";"),
                ));
                }
                if !matched {
                    result.push(format!(
                    "location={location:#x} store={label} entries={entries} ssl_root_match=none"
                ));
                }
            }
        }
        result
    }
}

pub use budget::{Budget, CancelCallback};
pub use error::Error;

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, DistinguishedName, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use windows_sys::Win32::Security::Cryptography::{
    CertCreateCertificateContext, CertFreeCertificateContext, CertGetNameStringW,
    CERT_NAME_ISSUER_FLAG, CERT_NAME_SIMPLE_DISPLAY_TYPE, X509_ASN_ENCODING,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

struct Endpoint {
    label: &'static str,
    host: &'static str,
    ip: &'static str,
}

const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        label: "cloudflare_ipv4",
        host: "cloudflare-dns.com",
        ip: "1.1.1.1",
    },
    Endpoint {
        label: "cloudflare_ipv6",
        host: "cloudflare-dns.com",
        ip: "2606:4700:4700::1111",
    },
    Endpoint {
        label: "google_ipv4",
        host: "dns.google",
        ip: "8.8.8.8",
    },
    Endpoint {
        label: "google_ipv6",
        host: "dns.google",
        ip: "2001:4860:4860::8888",
    },
];

const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(15);
const WORKER_TIMEOUT: Duration = Duration::from_secs(60);

fn deadline() -> u64 {
    unsafe { GetTickCount64() }.saturating_add(15_000)
}

fn inner_rustls_error(error: &std::io::Error) -> Option<&rustls::Error> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
}

fn certificate_name(der: &[u8], issuer: bool) -> String {
    let context =
        unsafe { CertCreateCertificateContext(X509_ASN_ENCODING, der.as_ptr(), der.len() as u32) };
    if context.is_null() {
        return "<decode-error>".to_owned();
    }
    let flags = if issuer { CERT_NAME_ISSUER_FLAG } else { 0 };
    let size = unsafe {
        CertGetNameStringW(
            context,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            flags,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    } as usize;
    if size == 0 {
        unsafe { CertFreeCertificateContext(context) };
        return "<name-error>".to_owned();
    }
    let mut buffer = vec![0u16; size];
    let written = unsafe {
        CertGetNameStringW(
            context,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            flags,
            std::ptr::null(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    unsafe { CertFreeCertificateContext(context) };
    if written <= 1 {
        return "<name-error>".to_owned();
    }
    String::from_utf16_lossy(&buffer[..written - 1])
}

fn peer_certificate_line(label: &str, der: &CertificateDer<'_>) -> String {
    let hash = Sha256::digest(der.as_ref());
    let hash = hash
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    format!(
        "peer_chain={label} subject={} issuer={} der_sha256={hash}",
        certificate_name(der.as_ref(), false),
        certificate_name(der.as_ref(), true),
    )
}

#[derive(Debug)]
struct RecordingVerifier {
    inner: Arc<dyn ServerCertVerifier>,
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        println!(
            "peer_chain_server_name={name:?} intermediate_count={}",
            intermediates.len()
        );
        println!("{}", peer_certificate_line("leaf", end));
        for (index, certificate) in intermediates.iter().enumerate() {
            println!(
                "{}",
                peer_certificate_line(&format!("intermediate[{index}]"), certificate)
            );
        }
        self.inner
            .verify_server_cert(end, intermediates, name, ocsp, now)
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

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

fn remaining_until(endpoint_deadline: Instant, worker_deadline: Instant) -> Option<Duration> {
    endpoint_deadline
        .min(worker_deadline)
        .checked_duration_since(Instant::now())
}

async fn handshake(
    endpoint: &Endpoint,
    config: Arc<ClientConfig>,
    worker_deadline: Instant,
) -> String {
    let endpoint_deadline = Instant::now() + ENDPOINT_TIMEOUT;
    let ip: IpAddr = match endpoint.ip.parse() {
        Ok(ip) => ip,
        Err(error) => return format!("parse_error={error:?}"),
    };
    let address = SocketAddr::new(ip, 443);
    let name = match ServerName::try_from(endpoint.host.to_owned()) {
        Ok(name) => name,
        Err(error) => return format!("server_name_error={error:?}"),
    };
    let tcp_budget = match remaining_until(endpoint_deadline, worker_deadline) {
        Some(budget) if !budget.is_zero() => budget,
        _ => return "worker_timeout=1 phase=tcp".to_owned(),
    };
    let tcp = match timeout(tcp_budget, TcpStream::connect(address)).await {
        Ok(Ok(tcp)) => tcp,
        Ok(Err(error)) => return format!("tcp_error={error:?}"),
        Err(_) => {
            let phase = if worker_deadline <= endpoint_deadline {
                "worker"
            } else {
                "endpoint"
            };
            return format!("{phase}_timeout=1 phase=tcp");
        }
    };
    let tls_budget = match remaining_until(endpoint_deadline, worker_deadline) {
        Some(budget) if !budget.is_zero() => budget,
        _ => return "endpoint_timeout=1 phase=tls".to_owned(),
    };
    match timeout(tls_budget, TlsConnector::from(config).connect(name, tcp)).await {
        Ok(Ok(stream)) => {
            let (_, connection) = stream.get_ref();
            format!(
                "tls_ok=1 protocol={:?} alpn={:?}",
                connection.protocol_version(),
                connection.alpn_protocol()
            )
        }
        Ok(Err(error)) => {
            let inner = inner_rustls_error(&error)
                .map(|value| format!("{value:?}"))
                .unwrap_or_else(|| "none".to_owned());
            format!("tls_error={error:?} rustls_inner={inner}")
        }
        Err(_) => {
            let phase = if worker_deadline <= endpoint_deadline {
                "worker"
            } else {
                "endpoint"
            };
            format!("{phase}_timeout=1 phase=tls")
        }
    }
}

fn main() {
    println!(
        "diagnostic_pid={} runtime_modules=not_checked_by_rust_binary product_source=direct_module_reuse variant={}",
        std::process::id(),
        env!("DOH_DIAGNOSTIC_VARIANT")
    );
    let snapshot = match trust::Snapshot::load(Budget::until(deadline())) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            println!("snapshot_or_verifier_error={error:?}");
            std::process::exit(2);
        }
    };
    let roots = trust::accepted_root_sha256(&snapshot);
    println!(
        "accepted_root_count={} accepted_root_sha256={}",
        roots.len(),
        roots.join(",")
    );
    for detail in trust::native_ssl_root_eligibility() {
        println!("ssl_root_policy={detail}");
    }
    let verifier = match snapshot.verifier(Budget::until(deadline())) {
        Ok(verifier) => verifier,
        Err(error) => {
            println!("snapshot_or_verifier_error={error:?}");
            std::process::exit(2);
        }
    };
    let mut client_config = match ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    {
        Ok(builder) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(RecordingVerifier { inner: verifier }))
            .with_no_client_auth(),
        Err(error) => {
            println!("client_config_error={error:?}");
            std::process::exit(2);
        }
    };
    client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let config = Arc::new(client_config);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            println!("runtime_error={error:?}");
            std::process::exit(3);
        }
    };
    let worker_deadline = Instant::now() + WORKER_TIMEOUT;
    for endpoint in ENDPOINTS {
        let result = if Instant::now() >= worker_deadline {
            "worker_timeout=1 phase=endpoint".to_owned()
        } else {
            runtime.block_on(handshake(endpoint, config.clone(), worker_deadline))
        };
        println!(
            "case={} host={} literal_ip={} port=443 result={}",
            endpoint.label, endpoint.host, endpoint.ip, result
        );
    }
}
