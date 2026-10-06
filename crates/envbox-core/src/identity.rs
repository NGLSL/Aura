//! Opt-in read views. These labels do not change accounts, adapters or registry data.
use crate::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityProfile {
    pub computer_name: Option<String>,
    pub user_name: Option<String>,
    pub mac_address: Option<String>,
    pub machine_guid: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_require_unambiguous_canonical_values() {
        let mut identity = IdentityProfile::default();
        for value in ["", "bad name", "-host", "host-", "abcdefghijklmnop", "中文"] {
            identity.computer_name = Some(value.into());
            assert!(identity.validate().is_err(), "{value}");
        }
        identity.computer_name = Some("aura-test".into());
        identity.user_name = Some("test.user_1".into());
        identity.mac_address = Some("02:11:22:33:44:55".into());
        identity.machine_guid = Some("12345678-1234-1234-1234-123456789abc".into());
        assert!(identity.validate().is_ok());
        for value in [
            "00:00:00:00:00:00",
            "01:11:22:33:44:55",
            "02:aa:22:33:44:55",
            "021122334455",
        ] {
            identity.mac_address = Some(value.into());
            assert!(identity.validate().is_err(), "{value}");
        }
    }
}

impl IdentityProfile {
    pub fn is_host(&self) -> bool {
        self.computer_name.is_none()
            && self.user_name.is_none()
            && self.mac_address.is_none()
            && self.machine_guid.is_none()
    }

    /// Stable order shared by IPC, environment fallback and Runtime observations.
    pub fn flat_fields(&self) -> Vec<(String, String)> {
        [
            ("computer_name", &self.computer_name),
            ("user_name", &self.user_name),
            ("mac_address", &self.mac_address),
            ("machine_guid", &self.machine_guid),
        ]
        .into_iter()
        .filter_map(|(name, value)| {
            value
                .as_ref()
                .map(|value| (format!("identity_{name}"), value.clone()))
        })
        .collect()
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        let invalid = |field| DomainError::InvalidProfile(format!("invalid identity {field}"));
        if let Some(value) = &self.computer_name {
            if value.is_empty()
                || value.len() > 15
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                || !value.as_bytes()[0].is_ascii_alphanumeric()
                || !value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
            {
                return Err(invalid("computer_name"));
            }
        }
        if let Some(value) = &self.user_name {
            if value.is_empty()
                || value.len() > 64
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            {
                return Err(invalid("user_name"));
            }
        }
        if let Some(value) = &self.mac_address {
            let parts: Vec<_> = value.split(':').collect();
            let bytes: Option<Vec<u8>> = parts
                .iter()
                .map(|part| {
                    if part.len() != 2
                        || !part
                            .bytes()
                            .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(&c))
                    {
                        None
                    } else {
                        u8::from_str_radix(part, 16).ok()
                    }
                })
                .collect();
            if !bytes.is_some_and(|bytes| {
                bytes.len() == 6
                    && bytes[0] & 1 == 0
                    && bytes.iter().any(|byte| *byte != 0)
                    && bytes.iter().any(|byte| *byte != 255)
            }) {
                return Err(invalid(
                    "mac_address (uppercase unicast XX:XX:XX:XX:XX:XX required)",
                ));
            }
        }
        if let Some(value) = &self.machine_guid {
            if !uuid::Uuid::parse_str(value)
                .is_ok_and(|id| !id.is_nil() && id.to_string() == *value)
            {
                return Err(invalid("machine_guid"));
            }
        }
        Ok(())
    }
}
