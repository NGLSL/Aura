//! Ordered Profile DNS configuration. Transport availability is a separate startup gate.
use crate::{DnsMode, DomainError};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

pub const MAX_DNS_UPSTREAMS: usize = 8;
fn port_53() -> u16 {
    53
}
fn port_853() -> u16 {
    853
}

/// TLS revocation policy is independent from strict DNS routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsTlsRevocation {
    #[default]
    Standard,
    StrictOffline,
}
impl DnsTlsRevocation {
    pub const ALL: [Self; 2] = [Self::Standard, Self::StrictOffline];
    pub fn wire_value(self) -> &'static str {
        match self {
            Self::Standard => "0",
            Self::StrictOffline => "1",
        }
    }
}
impl std::fmt::Display for DnsTlsRevocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Standard => "标准证书验证",
            Self::StrictOffline => "严格离线吊销验证",
        })
    }
}
impl std::str::FromStr for DnsTlsRevocation {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "standard" => Ok(Self::Standard),
            "strict_offline" => Ok(Self::StrictOffline),
            _ => Err("tls_revocation must be standard|strict_offline"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DnsUpstream {
    Udp {
        address: IpAddr,
        #[serde(default = "port_53")]
        port: u16,
    },
    Tcp {
        address: IpAddr,
        #[serde(default = "port_53")]
        port: u16,
    },
    Dot {
        address: IpAddr,
        #[serde(default = "port_853")]
        port: u16,
        server_name: String,
    },
    Doh {
        url: String,
        #[serde(default)]
        bootstrap_ips: Vec<IpAddr>,
        #[serde(default)]
        tls_revocation: DnsTlsRevocation,
    },
}

impl DnsUpstream {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Udp { address, port }
            | Self::Tcp { address, port }
            | Self::Dot { address, port, .. } => {
                validate_ip(*address)?;
                if *port == 0 {
                    return Err(invalid("port must be 1..65535"));
                }
                if let Self::Dot { server_name, .. } = self {
                    validate_identity(server_name)?;
                }
            }
            Self::Doh {
                url, bootstrap_ips, ..
            } => {
                let literal_authority = validate_doh_url(url)?;
                if !literal_authority && bootstrap_ips.is_empty() {
                    return Err(invalid("hostname DoH URL requires explicit bootstrap_ips"));
                }
                if bootstrap_ips.len() > MAX_DNS_UPSTREAMS {
                    return Err(invalid("at most eight bootstrap IPs are supported"));
                }
                for address in bootstrap_ips {
                    validate_ip(*address)?;
                }
            }
        }
        Ok(())
    }
    pub fn is_plaintext(&self) -> bool {
        matches!(self, Self::Udp { .. } | Self::Tcp { .. })
    }
    pub fn label(&self) -> String {
        match self {
            Self::Udp { address, port } => format!("UDP {address}:{port}"),
            Self::Tcp { address, port } => format!("TCP {address}:{port}"),
            Self::Dot {
                address,
                port,
                server_name,
            } => format!("DoT {address}:{port} · {server_name}"),
            Self::Doh {
                url,
                bootstrap_ips,
                tls_revocation,
            } => format!(
                "DoH {url} · bootstrap {} · {tls_revocation}",
                bootstrap_ips
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DnsProfile {
    pub mode: DnsMode,
    /// Compatibility projection for the old Runtime; empty if any upstream cannot be represented.
    pub servers: Vec<IpAddr>,
    pub upstreams: Vec<DnsUpstream>,
    pub strict: bool,
}
impl PartialEq for DnsProfile {
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode
            && self.strict == other.strict
            && self.effective_upstreams() == other.effective_upstreams()
    }
}
impl Eq for DnsProfile {}
impl Default for DnsProfile {
    fn default() -> Self {
        Self {
            mode: DnsMode::Host,
            servers: vec![],
            upstreams: vec![],
            strict: true,
        }
    }
}
impl DnsProfile {
    pub fn from_servers(mode: DnsMode, servers: Vec<IpAddr>) -> Self {
        let upstreams = servers
            .iter()
            .map(|address| DnsUpstream::Udp {
                address: *address,
                port: 53,
            })
            .collect();
        Self {
            mode,
            servers,
            upstreams,
            strict: true,
        }
    }
    pub fn typed(mode: DnsMode, strict: bool, upstreams: Vec<DnsUpstream>) -> Self {
        let servers = legacy_projection(&upstreams).unwrap_or_default();
        Self {
            mode,
            servers,
            upstreams,
            strict,
        }
    }
    pub fn effective_upstreams(&self) -> Vec<DnsUpstream> {
        if self.upstreams.is_empty() && !self.servers.is_empty() {
            self.servers
                .iter()
                .map(|address| DnsUpstream::Udp {
                    address: *address,
                    port: 53,
                })
                .collect()
        } else {
            self.upstreams.clone()
        }
    }
    pub fn validate(&self) -> Result<(), DomainError> {
        let upstreams = self.effective_upstreams();
        if self.mode == DnsMode::VirtualView && upstreams.is_empty() {
            return Err(invalid("DNS VirtualView requires at least one upstream"));
        }
        if upstreams.len() > MAX_DNS_UPSTREAMS {
            return Err(invalid("at most eight ordered upstreams are supported"));
        }
        if !self.upstreams.is_empty()
            && self.servers != legacy_projection(&self.upstreams).unwrap_or_default()
        {
            return Err(invalid("inconsistent legacy compatibility projection"));
        }
        for upstream in &upstreams {
            upstream.validate()?;
        }
        Ok(())
    }
    /// Current verified data-plane capability. Never substitute Host or drop unsupported fields.
    pub fn validate_runtime_support(&self) -> Result<(), DomainError> {
        self.validate()?;
        self.flat_fields()?;
        if self.mode == DnsMode::Host {
            return Ok(());
        }
        if !self.strict {
            return Err(invalid(
                "non-strict DNS fallback is not implemented by this Runtime",
            ));
        }
        Ok(())
    }
    /// Shared typed protocol v1 vocabulary; callers must also bound the whole PROFILE line.
    pub fn flat_fields(&self) -> Result<Vec<(String, String)>, DomainError> {
        self.validate()?;
        let upstreams = self.effective_upstreams();
        let mut fields = vec![
            ("dns_config_version".into(), "1".into()),
            (
                "dns_strict".into(),
                if self.strict { "1" } else { "0" }.into(),
            ),
            ("dns_upstream_count".into(), upstreams.len().to_string()),
        ];
        for (index, upstream) in upstreams.iter().enumerate() {
            let prefix = format!("dns_upstream_{index}_");
            let mut push =
                |key: &str, value: String| fields.push((format!("{prefix}{key}"), value));
            match upstream {
                DnsUpstream::Udp { address, port }
                | DnsUpstream::Tcp { address, port }
                | DnsUpstream::Dot { address, port, .. } => {
                    push(
                        "type",
                        match upstream {
                            DnsUpstream::Udp { .. } => "udp",
                            DnsUpstream::Tcp { .. } => "tcp",
                            _ => "dot",
                        }
                        .into(),
                    );
                    push("address", address.to_string());
                    push("port", port.to_string());
                    if let DnsUpstream::Dot { server_name, .. } = upstream {
                        push("server_name", server_name.clone());
                    }
                }
                DnsUpstream::Doh {
                    url,
                    bootstrap_ips,
                    tls_revocation,
                } => {
                    push("type", "doh".into());
                    push("url", url.clone());
                    push("tls_revocation", tls_revocation.wire_value().into());
                    push("bootstrap_count", bootstrap_ips.len().to_string());
                    for (bootstrap_index, address) in bootstrap_ips.iter().enumerate() {
                        push(&format!("bootstrap_{bootstrap_index}"), address.to_string());
                    }
                }
            }
        }
        if fields
            .iter()
            .map(|(key, value)| key.len() + value.len() + 4)
            .sum::<usize>()
            > 8192
        {
            return Err(invalid("typed DNS fields exceed PROFILE eight KiB bound"));
        }
        Ok(fields)
    }
}

fn legacy_projection(upstreams: &[DnsUpstream]) -> Option<Vec<IpAddr>> {
    upstreams
        .iter()
        .map(|upstream| match upstream {
            DnsUpstream::Udp { address, port: 53 } => Some(*address),
            _ => None,
        })
        .collect()
}
impl Serialize for DnsProfile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("DnsProfile", 3)?;
        state.serialize_field("mode", &self.mode)?;
        state.serialize_field("strict", &self.strict)?;
        state.serialize_field("upstreams", &self.effective_upstreams())?;
        state.end()
    }
}
impl<'de> Deserialize<'de> for DnsProfile {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            mode: DnsMode,
            strict: Option<bool>,
            servers: Option<Vec<IpAddr>>,
            upstreams: Option<Vec<DnsUpstream>>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let result = match (wire.servers, wire.upstreams) {
            (Some(_), Some(_)) => {
                return Err(serde::de::Error::custom(
                    "DNS servers and upstreams cannot be mixed",
                ))
            }
            (Some(servers), None) => {
                let mut profile = Self::from_servers(wire.mode, servers);
                profile.strict = wire.strict.unwrap_or(true);
                profile
            }
            (None, Some(upstreams)) => {
                Self::typed(wire.mode, wire.strict.unwrap_or(true), upstreams)
            }
            (None, None) => {
                return Err(serde::de::Error::custom(
                    "DNS servers or upstreams is required",
                ))
            }
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidProfile(format!("DNS: {reason}"))
}
fn validate_ip(address: IpAddr) -> Result<(), DomainError> {
    if address.is_unspecified() || address.is_multicast() {
        return Err(invalid(
            "DNS/bootstrap address must be a unicast literal IP",
        ));
    }
    Ok(())
}
fn validate_identity(identity: &str) -> Result<(), DomainError> {
    if let Ok(address) = identity.parse::<IpAddr>() {
        return validate_ip(address);
    }
    if identity.is_empty()
        || identity.len() > 253
        || !identity.is_ascii()
        || identity.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        })
    {
        return Err(invalid(
            "certificate identity must be an ASCII DNS name or literal IP",
        ));
    }
    Ok(())
}
fn validate_doh_url(url: &str) -> Result<bool, DomainError> {
    if url.len() > 2047
        || !url.is_ascii()
        || url.chars().any(|ch| ch.is_control() || ch.is_whitespace())
        || url.contains(['\\', '#', '"', '<', '>', '`'])
    {
        return Err(invalid("invalid or oversized DoH URL"));
    }
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| invalid("DoH requires https://"))?;
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.is_empty() || authority.contains(['@', '%']) {
        return Err(invalid(
            "DoH credentials and encoded authority are unsupported",
        ));
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, tail) = bracketed
            .split_once(']')
            .ok_or_else(|| invalid("invalid IPv6 URL authority"))?;
        host.parse::<std::net::Ipv6Addr>()
            .map_err(|_| invalid("invalid IPv6 URL authority"))?;
        let port = if tail.is_empty() {
            None
        } else {
            Some(
                tail.strip_prefix(':')
                    .ok_or_else(|| invalid("invalid DoH authority"))?,
            )
        };
        (host, port)
    } else {
        let mut parts = authority.split(':');
        let host = parts.next().unwrap_or("");
        let port = parts.next();
        if parts.next().is_some() {
            return Err(invalid("IPv6 URL authority must be bracketed"));
        }
        (host, port)
    };
    validate_identity(host)?;
    if let Some(port) = port {
        if port.parse::<u16>().ok().filter(|port| *port > 0).is_none() {
            return Err(invalid("invalid DoH URL port"));
        }
    }
    let bytes = url.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] == b'%'
            && (index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit())
        {
            return Err(invalid("invalid DoH URL percent encoding"));
        }
    }
    Ok(host.parse::<IpAddr>().is_ok())
}
