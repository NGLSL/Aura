//! Opt-in identity values. Empty inputs retain the host view; no host values are sampled.

use envbox_core::IdentityProfile;
use uuid::Uuid;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct IdentityDraft {
    pub computer_name: String,
    pub user_name: String,
    pub mac_address: String,
    pub machine_guid: String,
}

impl IdentityDraft {
    pub fn from_profile(profile: &IdentityProfile) -> Self {
        Self {
            computer_name: profile.computer_name.clone().unwrap_or_default(),
            user_name: profile.user_name.clone().unwrap_or_default(),
            mac_address: profile.mac_address.clone().unwrap_or_default(),
            machine_guid: profile.machine_guid.clone().unwrap_or_default(),
        }
    }

    pub fn to_profile(&self) -> IdentityProfile {
        let mac_address = optional(&self.mac_address).map(|value| value.to_ascii_uppercase());
        let machine_guid = optional(&self.machine_guid).map(|value| {
            Uuid::parse_str(&value)
                .map(|guid| guid.to_string())
                .unwrap_or(value)
        });
        IdentityProfile {
            computer_name: optional(&self.computer_name),
            user_name: optional(&self.user_name),
            mac_address,
            machine_guid,
        }
    }
}

fn optional(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_edit_roundtrip_and_clear_to_host() {
        let draft = IdentityDraft {
            computer_name: " AURA-US ".into(),
            user_name: " test.user ".into(),
            mac_address: " 02:aa:bb:cc:dd:ee ".into(),
            machine_guid: " {00112233-4455-6677-8899-AABBCCDDEEFF} ".into(),
        };
        let profile = draft.to_profile();
        assert_eq!(profile.computer_name.as_deref(), Some("AURA-US"));
        assert_eq!(profile.user_name.as_deref(), Some("test.user"));
        assert_eq!(profile.mac_address.as_deref(), Some("02:AA:BB:CC:DD:EE"));
        assert_eq!(
            profile.machine_guid.as_deref(),
            Some("00112233-4455-6677-8899-aabbccddeeff")
        );
        assert_eq!(IdentityDraft::from_profile(&profile).to_profile(), profile);
        let cleared = IdentityDraft {
            computer_name: " ".into(),
            user_name: "\t".into(),
            mac_address: "".into(),
            machine_guid: "\n".into(),
        };
        assert_eq!(cleared.to_profile(), IdentityProfile::default());
        assert_eq!(
            IdentityDraft::default().to_profile(),
            IdentityProfile::default()
        );
    }

    #[test]
    fn invalid_identity_input_remains_available_for_validation() {
        let draft = IdentityDraft {
            mac_address: "not-a-mac".into(),
            machine_guid: "not-a-guid".into(),
            ..Default::default()
        };
        let profile = draft.to_profile();
        assert_eq!(profile.mac_address.as_deref(), Some("NOT-A-MAC"));
        assert_eq!(profile.machine_guid.as_deref(), Some("not-a-guid"));
        assert!(profile.validate().is_err());
    }

    #[test]
    fn saved_profile_loads_identity_draft_and_cleared_edit_retains_host() {
        use envbox_core::{EnvironmentProfile, LocaleProfile, TimezoneProfile};
        use envbox_storage::{ConfigStore, ProfileDocument};

        let root = std::env::temp_dir().join(format!("aura-gui-identity-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let mut profile = EnvironmentProfile {
            id: Uuid::new_v4(),
            name: "Identity view".into(),
            locale: LocaleProfile {
                locale_name: "en-US".into(),
                ui_language: "en-US".into(),
                region: "US".into(),
            },
            timezone: TimezoneProfile {
                windows_id: "Pacific Standard Time".into(),
                iana_id: "America/Los_Angeles".into(),
            },
            dns: Default::default(),
            environment: Default::default(),
            registry: Default::default(),
            browser: Default::default(),
            identity: IdentityDraft {
                computer_name: "AURA-US".into(),
                user_name: "test.user".into(),
                mac_address: "02:aa:bb:cc:dd:ee".into(),
                machine_guid: "00112233-4455-6677-8899-AABBCCDDEEFF".into(),
            }
            .to_profile(),
        };
        store
            .save_profiles(&ProfileDocument {
                profiles: vec![profile.clone()],
            })
            .unwrap();
        let loaded = store.load_profiles().unwrap().profiles.remove(0);
        let draft = crate::app::profile_to_draft(&loaded);
        assert_eq!(draft.identity.to_profile(), profile.identity);
        let mut edited = draft.clone();
        edited.identity = IdentityDraft::default();
        assert_eq!(draft.identity.to_profile(), profile.identity);
        profile.identity = edited.identity.to_profile();
        store
            .save_profiles(&ProfileDocument {
                profiles: vec![profile],
            })
            .unwrap();
        let reloaded = store.load_profiles().unwrap().profiles.remove(0);
        assert!(crate::app::profile_to_draft(&reloaded)
            .identity
            .to_profile()
            .is_host());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
