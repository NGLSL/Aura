//! Unsaved proxy input shared by Profile saving and egress lookup.
use envbox_core::{BrowserProxy, BrowserProxyKind};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProxyChoice {
    #[default]
    None,
    Http,
    Socks5,
}
impl ProxyChoice {
    pub const ALL: [Self; 3] = [Self::None, Self::Http, Self::Socks5];
}
impl std::fmt::Display for ProxyChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "无 Profile 代理",
            Self::Http => "HTTP",
            Self::Socks5 => "SOCKS5",
        })
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyDraft {
    pub choice: ProxyChoice,
    pub host: String,
    pub port: String,
}
impl ProxyDraft {
    pub fn from_profile(proxy: Option<&BrowserProxy>) -> Self {
        match proxy {
            None => Self::default(),
            Some(p) => Self {
                choice: match p.kind {
                    BrowserProxyKind::Http => ProxyChoice::Http,
                    BrowserProxyKind::Socks5 => ProxyChoice::Socks5,
                },
                host: p.host.clone(),
                port: p.port.to_string(),
            },
        }
    }
    pub fn to_profile(&self) -> Result<Option<BrowserProxy>, String> {
        let kind = match self.choice {
            ProxyChoice::None => return Ok(None),
            ProxyChoice::Http => BrowserProxyKind::Http,
            ProxyChoice::Socks5 => BrowserProxyKind::Socks5,
        };
        let proxy = BrowserProxy {
            kind,
            host: self.host.clone(),
            port: self
                .port
                .parse()
                .map_err(|_| "代理端口需要 1–65535 的整数".to_string())?,
        };
        proxy.validate().map_err(|e| e.to_string())?;
        Ok(Some(proxy))
    }
    pub fn source_label(&self) -> String {
        match self.choice {
            ProxyChoice::None => "Aura 当前网络（无 Profile 代理）".into(),
            _ => format!("{} 代理 · {}:{}", self.choice, self.host, self.port),
        }
    }
}
