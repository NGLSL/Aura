use envbox_core::{DnsMode, DnsProfile, DnsTlsRevocation, DnsUpstream};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportChoice {
    #[default]
    Udp,
    Tcp,
    Dot,
    Doh,
}
impl TransportChoice {
    pub const ALL: [Self; 4] = [Self::Udp, Self::Tcp, Self::Dot, Self::Doh];
}
impl std::fmt::Display for TransportChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Udp => "UDP（明文）",
            Self::Tcp => "TCP（明文）",
            Self::Dot => "DoT",
            Self::Doh => "DoH",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct UpstreamDraft {
    pub transport: TransportChoice,
    pub address: String,
    pub port: String,
    pub server_name: String,
    pub url: String,
    pub bootstrap: String,
    pub tls_revocation: DnsTlsRevocation,
}
impl Default for UpstreamDraft {
    fn default() -> Self {
        Self {
            transport: TransportChoice::Udp,
            address: String::new(),
            port: "53".into(),
            server_name: String::new(),
            url: String::new(),
            bootstrap: String::new(),
            tls_revocation: DnsTlsRevocation::Standard,
        }
    }
}
impl UpstreamDraft {
    fn has_input(&self) -> bool {
        !self.address.is_empty()
            || !self.server_name.is_empty()
            || !self.url.is_empty()
            || !self.bootstrap.is_empty()
    }
    fn build(&self) -> Result<DnsUpstream, String> {
        let upstream = if self.transport == TransportChoice::Doh {
            let bootstrap_ips = self
                .bootstrap
                .split([',', ';', ' '])
                .filter(|part| !part.is_empty())
                .map(|part| {
                    part.parse()
                        .map_err(|_| "bootstrap 必须为 literal IP".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            DnsUpstream::Doh {
                url: self.url.clone(),
                bootstrap_ips,
                tls_revocation: self.tls_revocation,
            }
        } else {
            let address = self
                .address
                .parse()
                .map_err(|_| "上游地址必须为 literal IP")?;
            let port = self.port.parse().map_err(|_| "端口必须为 1–65535")?;
            match self.transport {
                TransportChoice::Udp => DnsUpstream::Udp { address, port },
                TransportChoice::Tcp => DnsUpstream::Tcp { address, port },
                _ => DnsUpstream::Dot {
                    address,
                    port,
                    server_name: self.server_name.clone(),
                },
            }
        };
        upstream.validate().map_err(|err| err.to_string())?;
        Ok(upstream)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DnsEditor {
    pub upstreams: Vec<DnsUpstream>,
    pub strict: bool,
    pub draft: UpstreamDraft,
}
impl Default for DnsEditor {
    fn default() -> Self {
        Self {
            upstreams: vec![],
            strict: true,
            draft: Default::default(),
        }
    }
}
impl DnsEditor {
    pub fn from_profile(profile: &DnsProfile) -> Self {
        Self {
            upstreams: profile.effective_upstreams(),
            strict: profile.strict,
            draft: Default::default(),
        }
    }
    pub fn add(&mut self) -> Result<(), String> {
        let mut upstreams = self.upstreams.clone();
        upstreams.push(self.draft.build()?);
        DnsProfile::typed(DnsMode::Host, self.strict, upstreams.clone())
            .validate()
            .map_err(|err| err.to_string())?;
        self.upstreams = upstreams;
        self.draft = Default::default();
        Ok(())
    }
    pub fn move_upstream(&mut self, index: usize, upward: bool) {
        let target = if upward {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        if let Some(target) = target.filter(|target| *target < self.upstreams.len()) {
            if index < self.upstreams.len() {
                self.upstreams.swap(index, target);
            }
        }
    }
    pub fn draft_error(&self) -> Option<String> {
        self.draft
            .has_input()
            .then(|| self.draft.build().err())
            .flatten()
    }
    pub fn to_profile(&self, mode: DnsMode) -> Result<DnsProfile, String> {
        let mut upstreams = self.upstreams.clone();
        if self.draft.has_input() {
            upstreams.push(self.draft.build()?);
        }
        let profile = DnsProfile::typed(mode, self.strict, upstreams);
        profile.validate().map_err(|err| err.to_string())?;
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostname_doh_without_manual_bootstrap_can_be_saved() {
        let mut editor = DnsEditor::default();
        editor.draft.transport = TransportChoice::Doh;
        editor.draft.url = "https://resolver.example/dns-query".into();
        let profile = editor.to_profile(DnsMode::VirtualView).unwrap();
        profile.validate_runtime_support().unwrap();
        assert!(
            matches!(&profile.upstreams[0], DnsUpstream::Doh { bootstrap_ips, .. } if bootstrap_ips.is_empty())
        );
        let loaded: DnsProfile = toml::from_str(&toml::to_string(&profile).unwrap()).unwrap();
        assert_eq!(loaded, profile);
    }
    #[test]
    fn save_includes_pending_upstream_and_reload_does_not_duplicate_it() {
        let mut editor = DnsEditor::default();
        editor.draft.address = "1.1.1.1".into();
        editor.add().unwrap();
        editor.draft.transport = TransportChoice::Doh;
        editor.draft.url = "https://resolver.example/dns-query".into();
        let saved = editor.to_profile(DnsMode::VirtualView).unwrap();
        assert_eq!(saved.upstreams.len(), 2);
        assert!(matches!(saved.upstreams[0], DnsUpstream::Udp { .. }));
        assert!(matches!(saved.upstreams[1], DnsUpstream::Doh { .. }));
        let reloaded = DnsEditor::from_profile(&saved);
        assert_eq!(reloaded.to_profile(DnsMode::VirtualView).unwrap(), saved);
        editor.draft.url = "http://resolver.example/dns-query".into();
        assert!(editor.draft_error().is_some());
        assert!(editor.to_profile(DnsMode::VirtualView).is_err());
        assert_eq!(editor.upstreams.len(), 1);
    }
    #[test]
    fn gui_add_sort_and_roundtrip_preserve_identity_and_strict() {
        let mut editor = DnsEditor::default();
        editor.draft.address = "1.1.1.1".into();
        editor.add().unwrap();
        editor.draft.transport = TransportChoice::Doh;
        editor.draft.url = "http://dns.example/dns-query".into();
        assert!(editor.add().is_err());
        assert_eq!(editor.upstreams.len(), 1);
        editor.draft.url = "https://dns.example/dns-query".into();
        editor.draft.bootstrap = "1.0.0.1".into();
        editor.draft.tls_revocation = DnsTlsRevocation::StrictOffline;
        editor.add().unwrap();
        editor.move_upstream(1, true);
        let profile = editor.to_profile(DnsMode::VirtualView).unwrap();
        assert!(matches!(profile.upstreams[0], DnsUpstream::Doh { .. }));
        assert!(profile.strict);
        assert!(matches!(
            profile.upstreams[0],
            DnsUpstream::Doh {
                tls_revocation: DnsTlsRevocation::StrictOffline,
                ..
            }
        ));
        profile.validate_runtime_support().unwrap();
        let loaded: DnsProfile = toml::from_str(&toml::to_string(&profile).unwrap()).unwrap();
        assert_eq!(DnsEditor::from_profile(&loaded).upstreams, editor.upstreams);
        editor.strict = false;
        assert!(!editor.to_profile(DnsMode::VirtualView).unwrap().strict);
        editor.draft.address = "not-an-IP".into();
        assert!(editor.to_profile(DnsMode::VirtualView).is_err());
    }
}
