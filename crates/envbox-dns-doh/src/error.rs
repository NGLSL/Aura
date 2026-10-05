#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    None = 0,
    Argument = 1,
    Cancelled = 2,
    Deadline = 3,
    Network = 4,
    Tls = 5,
    Certificate = 6,
    RevocationUnknown = 7,
    Revoked = 8,
    Identity = 9,
    Disallowed = 10,
    TrustSnapshot = 11,
    HttpStatus = 12,
    MediaType = 13,
    BodyLimit = 14,
    Http = 15,
    Panic = 16,
    ContentEncoding = 17,
}

impl From<rustls::Error> for Error {
    fn from(error: rustls::Error) -> Self {
        use rustls::CertificateError as C;
        match error {
            rustls::Error::InvalidCertificate(C::Revoked) => Self::Revoked,
            rustls::Error::InvalidCertificate(
                C::UnknownRevocationStatus
                | C::ExpiredRevocationList
                | C::ExpiredRevocationListContext { .. },
            ) => Self::RevocationUnknown,
            rustls::Error::InvalidCertificate(
                C::NotValidForName | C::NotValidForNameContext { .. },
            ) => Self::Identity,
            rustls::Error::InvalidCertificate(C::ApplicationVerificationFailure) => {
                Self::Disallowed
            }
            rustls::Error::InvalidCertificate(_) => Self::Certificate,
            _ => Self::Tls,
        }
    }
}
