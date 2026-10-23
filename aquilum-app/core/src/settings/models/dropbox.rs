//! Public Dropbox read-only configuration and legacy settings tests.
use serde::{Deserialize, Serialize};

/// Dropbox 3.13 read-only test configuration. All fields are public
/// identifiers or an out-of-band BLAKE3 pin; OAuth secrets are held only by
/// the OS credential vault in the separate Dropbox connector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DropboxReadOnlyDraft {
    pub oauth_app_key: String,
    pub registered_port: u16,
    pub account_id: String,
    pub root_namespace_id: String,
    pub vault_id: String,
    pub profile_id: String,
    pub trusted_head_blake3: String,
}
impl Default for DropboxReadOnlyDraft {
    fn default() -> Self {
        Self {
            oauth_app_key: String::new(),
            registered_port: 47219,
            account_id: String::new(),
            root_namespace_id: String::new(),
            vault_id: String::new(),
            profile_id: String::new(),
            trusted_head_blake3: String::new(),
        }
    }
}
impl DropboxReadOnlyDraft {
    fn label(value: &str, max: usize) -> bool {
        !value.is_empty() && value.len() <= max && value.bytes().all(|c|
            c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    }
    pub fn valid_app_key(value: &str) -> bool {
        value.is_empty() || Self::label(value, 128)
    }
    pub fn valid_account(value: &str) -> bool {
        value.is_empty() || value.strip_prefix("dbid:")
            .is_some_and(|rest| Self::label(rest, 128))
    }
    pub fn valid_namespace(value: &str) -> bool {
        value.is_empty() || (!value.is_empty() && value.len() <= 32
            && value.bytes().all(|c| c.is_ascii_digit()))
    }
    pub fn valid_vault(value: &str) -> bool {
        if value.is_empty() { return true; }
        value.len() == 36 && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) { c == b'-' }
            else { c.is_ascii_digit() || (b'a'..=b'f').contains(&c) }
        })
    }
    pub fn valid_profile(value: &str) -> bool {
        value.is_empty() || Self::label(value, 64)
    }
    pub fn valid_pin(value: &str) -> bool {
        value.is_empty() || (value.len() == 64
            && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            && value.bytes().any(|c| c != b'0'))
    }
    pub fn ready(&self) -> bool {
        !self.oauth_app_key.is_empty() && self.registered_port != 0
            && !self.account_id.is_empty() && !self.root_namespace_id.is_empty()
            && !self.vault_id.is_empty() && !self.profile_id.is_empty()
            && !self.trusted_head_blake3.is_empty()
            && Self::valid_app_key(&self.oauth_app_key)
            && Self::valid_account(&self.account_id)
            && Self::valid_namespace(&self.root_namespace_id)
            && Self::valid_vault(&self.vault_id)
            && Self::valid_profile(&self.profile_id)
            && Self::valid_pin(&self.trusted_head_blake3)
    }
}

#[cfg(test)]
mod dropbox_readonly_draft_tests {
    use super::DropboxReadOnlyDraft;
    use crate::settings::models::AppConfig;
    #[test]
    fn legacy_settings_default_to_disconnected() {
        let old: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(!old.dropbox_readonly.ready());
        assert_eq!(old.dropbox_readonly.registered_port, 47219);
    }
    #[test]
    fn only_public_identifiers_and_independent_pins_are_serialized() {
        let mut config = AppConfig::default();
        let d = &mut config.dropbox_readonly;
        d.oauth_app_key = "app_key_123".into();
        d.account_id = "dbid:alice".into();
        d.root_namespace_id = "12345".into();
        d.vault_id = "12345678-1234-1234-1234-123456789abc".into();
        d.profile_id = "test".into();
        d.trusted_head_blake3 = "a".repeat(64);
        assert!(d.ready());
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("accessToken") && !json.contains("refreshToken"));
        assert!(!json.contains("clientSecret") && !json.contains("authorizationCode"));
        assert_eq!(serde_json::from_str::<AppConfig>(&json).unwrap()
            .dropbox_readonly, config.dropbox_readonly);
    }
    #[test]
    fn pasted_urls_secrets_and_zero_pin_are_not_accepted_as_configuration() {
        assert!(!DropboxReadOnlyDraft::valid_app_key("https://example.com"));
        assert!(!DropboxReadOnlyDraft::valid_account("Bearer abc"));
        assert!(!DropboxReadOnlyDraft::valid_namespace("token=123"));
        assert!(!DropboxReadOnlyDraft::valid_vault("../other-vault"));
        assert!(!DropboxReadOnlyDraft::valid_profile("other/profile"));
        assert!(!DropboxReadOnlyDraft::valid_pin(&"0".repeat(64)));
    }
}

