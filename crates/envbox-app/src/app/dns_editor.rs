use envbox_core::{DnsMode, DnsProfile, DnsUpstream};

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
    pub fn to_profile(&self, mode: DnsMode) -> Result<DnsProfile, String> {
        if self.draft.has_input() {
            return Err("请先添加或清空正在编辑的 DNS 上游".into());
        }
        let profile = DnsProfile::typed(mode, self.strict, self.upstreams.clone());
        profile.validate().map_err(|err| err.to_string())?;
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_add_sort_and_roundtrip_preserve_identity_and_strict() {
        let mut editor = DnsEditor::default();
        editor.draft.address = "1.1.1.1".into();
        editor.add().unwrap();
        editor.draft.transport = TransportChoice::Doh;
        editor.draft.url = "https://dns.example/dns-query".into();
        assert!(editor.add().is_err());
        assert_eq!(editor.upstreams.len(), 1);
        editor.draft.bootstrap = "1.0.0.1".into();
        editor.add().unwrap();
        editor.move_upstream(1, true);
        let profile = editor.to_profile(DnsMode::VirtualView).unwrap();
        assert!(matches!(profile.upstreams[0], DnsUpstream::Doh { .. }));
        assert!(profile.strict);
        assert!(profile.validate_runtime_support().is_err());
        let loaded: DnsProfile = toml::from_str(&toml::to_string(&profile).unwrap()).unwrap();
        assert_eq!(DnsEditor::from_profile(&loaded).upstreams, editor.upstreams);
        editor.strict = false;
        assert!(!editor.to_profile(DnsMode::VirtualView).unwrap().strict);
        editor.draft.address = "not-an-IP".into();
        assert!(editor.to_profile(DnsMode::VirtualView).is_err());
    }
}
