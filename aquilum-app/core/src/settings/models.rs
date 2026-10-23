use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bm25fParams {
    pub k1: f32,
    pub k3: f32,
    pub b_title: f32,
    pub b_body: f32,
    pub title_weight: f32,
}

impl Default for Bm25fParams {
    fn default() -> Self {
        Self {
            k1: 1.2,
            k3: 8.0,
            b_title: 0.3,
            b_body: 0.75,
            title_weight: 2.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnalysisSettings {
    pub enable_bm25f: bool,
    pub enable_adamic_adar: bool,
    pub enable_wikixiv: bool,
    pub bm25f_params: Bm25fParams,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            enable_bm25f: true,
            enable_adamic_adar: true,
            enable_wikixiv: true,
            bm25f_params: Bm25fParams::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchIndexSettings {
    pub candidate_pool_size: usize,
    pub max_query_terms: usize,
}

impl Default for SearchIndexSettings {
    fn default() -> Self {
        Self {
            candidate_pool_size: 256,
            max_query_terms: 64,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FontSettings {
    pub font_family: String,
    pub font_weight: u32,
    pub font_size_base: u32,
}

impl Default for FontSettings {
    fn default() -> Self {
        UiSettings::default().font
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EditorSettings {
    pub save_debounce_ms: u32,
    pub line_height: f32,
    pub max_width_ch: u32,
    pub smart_dashes: bool,
    pub list_callouts: bool,
    pub colored_tags: bool,
    pub selection_matches: bool,
    pub auto_link_title: bool,
    pub live_tabs: u32,
    pub link_suggest: bool,
    pub link_suggest_min_chars: u32,
    #[serde(flatten)]
    pub font: FontSettings,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            save_debounce_ms: 1000,
            line_height: 1.8,
            max_width_ch: 65,
            smart_dashes: true,
            list_callouts: true,
            colored_tags: true,
            selection_matches: false,
            auto_link_title: true,
            live_tabs: 3,
            link_suggest: true,
            link_suggest_min_chars: 2,
            font: FontSettings {
                font_family: "iA Writer Mono".to_string(),
                font_weight: 400,
                font_size_base: 16,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReaderSettings {
    pub line_height: f32,
    pub max_width_ch: u32,
    pub margin_px: u32,
    pub justify: bool,
    pub hyphenate: bool,
    pub flow: String,
    #[serde(flatten)]
    pub font: FontSettings,
}

impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            line_height: 1.6,
            max_width_ch: 65,
            margin_px: 48,
            justify: true,
            hyphenate: true,
            flow: "paginated".to_string(),
            font: FontSettings {
                font_family: "iA Writer Quattro".to_string(),
                font_weight: 400,
                font_size_base: 18,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiSettings {
    pub theme: String,
    pub language: String,
    pub primary_color: String,
    pub animations: bool,
    #[serde(flatten)]
    pub font: FontSettings,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            language: String::new(),
            primary_color: "#1471eb".to_string(),
            animations: true,
            font: FontSettings {
                font_family: "Inter".to_string(),
                font_weight: 400,
                font_size_base: 14,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrashSettings {
    pub retention_days: u32,
}

impl Default for TrashSettings {
    fn default() -> Self {
        Self {
            retention_days: 30,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistorySettings {
    pub retention_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    pub allow_write: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8787,
            token: String::new(),
            allow_write: true,
        }
    }
}

/// Public, non-secret setup draft only. This never grants an S3 capability,
/// stores credentials, binds a remote vault or enables a sync worker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct YandexS3Onboarding {
    pub bucket: String,
    /// The name of an external named AWS credentials profile, never its keys.
    pub aws_profile_name: String,
}

impl YandexS3Onboarding {
    /// Reject accidentally pasted access keys, URLs and other arbitrary data.
    /// This is a *draft validation*, not proof that an account owns the bucket.
    pub fn valid_bucket(value: &str) -> bool {
        if value.is_empty() {
            return true;
        }
        let b = value.as_bytes();
        (3..=63).contains(&b.len())
            && b.first().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && b.last().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
            && !value.contains("--")
    }

    pub fn valid_profile_name(value: &str) -> bool {
        if value.is_empty() {
            return true;
        }
        // Restrict this to a short, lowercase *alias*. Access Key IDs and
        // Secret Access Keys are not valid profile names in this UI.
        (3..=32).contains(&value.len())
            && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && value.bytes().all(|c| c.is_ascii_lowercase()
                || c.is_ascii_digit() || c == b'_' || c == b'-')
    }
}

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
    use super::{AppConfig, DropboxReadOnlyDraft};
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

/// Public-only preparation for *the user's own server*. No SSH private
/// key, password, token, sudo credential or remote installation authority is
/// accepted or serialized. Enter the public host key AND its SHA256 value
/// from an independent provider console; an SSH peer cannot self-enroll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerBootstrapDraft {
    pub host: String,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub host_public_key: String,
    /// OpenSSH SHA256:<base64-no-padding> over canonical ssh-ed25519 wire.
    pub host_key_fingerprint: String,
}
impl Default for ServerBootstrapDraft {
    fn default() -> Self {
        Self {
            host: String::new(),
            ssh_user: String::new(),
            ssh_port: 22,
            host_public_key: String::new(),
            host_key_fingerprint: String::new(),
        }
    }
}
impl ServerBootstrapDraft {
    pub fn valid_host(s: &str) -> bool {
        if s.is_empty() { return true; }
        if s.len() > 253 || !s.is_ascii() || s.contains("..") { return false; }
        s.split('.').all(|label| {
            !label.is_empty() && label.len() <= 63
                && label.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                && label.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
                && label.bytes().all(|c|
                    c.is_ascii_alphanumeric() || c == b'-')
        })
    }
    pub fn valid_user(s: &str) -> bool {
        if s.is_empty() { return true; }
        s != "root" && s.len() <= 32 && s.as_bytes().first().is_some_and(|c|
            c.is_ascii_lowercase() || *c == b'_')
            && s.bytes().all(|c| c.is_ascii_lowercase()
                || c.is_ascii_digit() || c == b'_' || c == b'-')
    }
    pub fn valid_port(port: u16) -> bool { port != 0 }

    /// Parse canonical 51-byte OpenSSH ed25519 host key. Exact two-field
    /// format excludes pasted private keys, comments, options and shell text.
    fn wire(value: &str) -> Option<[u8; 51]> {
        let mut parts = value.split_ascii_whitespace();
        if parts.next() != Some("ssh-ed25519") { return None; }
        let b64 = parts.next()?;
        if parts.next().is_some() || b64.len() != 68 { return None; }
        fn digit(v: u8) -> Option<u32> {
            match v {
                b'A'..=b'Z' => Some((v - b'A') as u32),
                b'a'..=b'z' => Some((v - b'a' + 26) as u32),
                b'0'..=b'9' => Some((v - b'0' + 52) as u32),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        }
        let mut wire = [0u8; 51];
        for (i, group) in b64.as_bytes().chunks_exact(4).enumerate() {
            let (a,b,c,d) = (
                digit(group[0])?, digit(group[1])?,
                digit(group[2])?, digit(group[3])?,
            );
            let offset = i * 3;
            wire[offset] = ((a << 2) | (b >> 4)) as u8;
            wire[offset+1] = ((b << 4) | (c >> 2)) as u8;
            wire[offset+2] = ((c << 6) | d) as u8;
        }
        if &wire[..4] != b"\0\0\0\x0b"
            || &wire[4..15] != b"ssh-ed25519"
            || &wire[15..19] != b"\0\0\0\x20"
        { return None; }
        Some(wire)
    }
    pub fn valid_public_key(value: &str) -> bool {
        value.is_empty() || Self::wire(value).is_some()
    }
    pub fn valid_fingerprint(value: &str) -> bool {
        if value.is_empty() { return true; }
        value.len() == 50 && value.starts_with("SHA256:")
            && value.as_bytes()[7..].iter().all(|c|
                c.is_ascii_alphanumeric() || matches!(*c, b'+' | b'/'))
    }
    fn b64_no_pad(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::with_capacity((bytes.len() * 4 + 2) / 3);
        for part in bytes.chunks(3) {
            let a = part[0];
            let b = part.get(1).copied().unwrap_or(0);
            let c = part.get(2).copied().unwrap_or(0);
            result.push(TABLE[(a >> 2) as usize] as char);
            result.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
            if part.len() >= 2 {
                result.push(TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char);
            }
            if part.len() == 3 {
                result.push(TABLE[(c & 63) as usize] as char);
            }
        }
        result
    }
    /// Map the *offline-only* native draft to the portable B0 typed target.
    /// The result is NOT the peer identity learned in an SSH key exchange
    /// and grants no installation privilege. Preview transport must still
    /// reverify signed host KEX against this exact SHA-256.
    #[cfg(feature = "server-bootstrap-b0-lab")]
    pub fn to_unconnected_b0_target(
        &self,
    ) -> Option<crate::server_bootstrap_lab::ServerTarget> {
        if !self.valid_offline_pin() { return None; }
        let wire = Self::wire(&self.host_public_key)?;
        let sha: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, &wire)
            .as_ref().try_into().ok()?;
        let target = crate::server_bootstrap_lab::ServerTarget {
            host: self.host.clone(),
            ssh_port: self.ssh_port,
            ssh_user: self.ssh_user.clone(),
            trusted_key_sha256: sha,
        };
        target.validate().ok()?;
        Some(target)
    }
    /// TRUE means the operator-entered independent host-key pair is coherent
    /// for a future read-only preflight; it does NOT authenticate any server
    /// or authorize privileged installation.
    pub fn valid_offline_pin(&self) -> bool {
        if self.host.is_empty() || self.ssh_user.is_empty()
            || !Self::valid_host(&self.host)
            || !Self::valid_user(&self.ssh_user)
            || !Self::valid_port(self.ssh_port)
            || !Self::valid_fingerprint(&self.host_key_fingerprint)
        { return false; }
        let Some(wire) = Self::wire(&self.host_public_key) else { return false };
        let sha = ring::digest::digest(&ring::digest::SHA256, &wire);
        let expected = format!("SHA256:{}", Self::b64_no_pad(sha.as_ref()));
        ring::constant_time::verify_slices_are_equal(
            expected.as_bytes(), self.host_key_fingerprint.as_bytes(),
        ).is_ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub analysis: AnalysisSettings,
    pub mcp: McpSettings,
    pub trash: TrashSettings,
    pub history: HistorySettings,
    pub search: SearchIndexSettings,
    pub editor: EditorSettings,
    pub reader: ReaderSettings,
    pub ui: UiSettings,
    pub templates: TemplateSettings,
    pub files: FilesSettings,
    pub updates: UpdateSettings,
    pub s3_onboarding: YandexS3Onboarding,
    pub dropbox_readonly: DropboxReadOnlyDraft,
    pub server_setup: ServerBootstrapDraft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TemplateSettings {
    pub folder: String,
}

impl Default for TemplateSettings {
    fn default() -> Self {
        Self { folder: "Templates".to_string() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilesSettings {
    pub folder: String,
}

impl Default for FilesSettings {
    fn default() -> Self {
        Self { folder: "Files".to_string() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateSettings {
    pub auto: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { auto: true }
    }
}

#[cfg(test)]
mod yandex_onboarding_tests {
    use super::{AppConfig, YandexS3Onboarding};

    #[test]
    fn onboarding_fields_are_nonsecret_and_legacy_json_works() {
        let mut config: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(config.s3_onboarding.bucket.is_empty());
        assert!(config.s3_onboarding.aws_profile_name.is_empty());
        config.s3_onboarding.bucket = "book-archive-2026".into();
        config.s3_onboarding.aws_profile_name = "akula_yandex".into();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("book-archive-2026"));
        assert!(json.contains("awsProfileName"));
        assert!(!json.contains("secretAccessKey"));
        assert!(!json.contains("accessKeyId"));
        assert_eq!(serde_json::from_str::<AppConfig>(&json).unwrap()
            .s3_onboarding, config.s3_onboarding);
    }

    #[test]
    fn user_editable_draft_does_not_accept_urls_or_credential_payloads() {
        for valid in ["", "abc", "s3-vault-2026", "0private-9"] {
            assert!(YandexS3Onboarding::valid_bucket(valid), "{valid}");
        }
        for invalid in ["x", "https://storage.yandexcloud.net", "my.bucket",
                        "BUCKET", "secret_key=this-is-not-a-bucket", "spaces inside"] {
            assert!(!YandexS3Onboarding::valid_bucket(invalid), "{invalid}");
        }
        for valid in ["", "default", "akula_yandex", "profile-01"] {
            assert!(YandexS3Onboarding::valid_profile_name(valid));
        }
        for invalid in ["../../.aws/credentials", "Access Key ID", "aws;rm -rf /",
                        "profile=secret", "приватный_ключ", "ABCD0123456789ABCD",
                        "this-alias-is-way-too-long-to-be-a-profile"] {
            assert!(!YandexS3Onboarding::valid_profile_name(invalid), "{invalid}");
        }
    }
}


#[cfg(test)]
mod server_setup_tests {
    use super::{AppConfig, ServerBootstrapDraft};
    #[test]
    fn independent_pinned_public_host_identity_validates_without_network() {
        let mut wire = Vec::from(&b"\0\0\0\x0bssh-ed25519\0\0\0\x20"[..]);
        wire.extend_from_slice(&[7u8;32]);
        assert_eq!(wire.len(),51);
        let public = format!("ssh-ed25519 {}",ServerBootstrapDraft::b64_no_pad(&wire));
        let hash = ring::digest::digest(&ring::digest::SHA256,&wire);
        let fingerprint = format!("SHA256:{}",
            ServerBootstrapDraft::b64_no_pad(hash.as_ref()));
        let mut draft = ServerBootstrapDraft {
            host: "vps.example.org".into(),
            ssh_user: "ubuntu".into(),
            ssh_port: 22,
            host_public_key: public,
            host_key_fingerprint: fingerprint,
        };
        assert!(draft.valid_offline_pin());
        #[cfg(feature = "server-bootstrap-b0-lab")]
        {
            let target = draft.to_unconnected_b0_target().unwrap();
            assert_eq!(target.host, "vps.example.org");
            assert_eq!(target.ssh_user, "ubuntu");
            assert_eq!(target.ssh_port, 22);
            assert_eq!(target.trusted_key_sha256.as_slice(), hash.as_ref());
        }
        let replacement = if &draft.host_key_fingerprint[9..10] == "A" { "B" } else { "A" };
        draft.host_key_fingerprint.replace_range(9..10, replacement);
        assert!(!draft.valid_offline_pin(), "independently supplied digest mismatch");
        assert!(!ServerBootstrapDraft::valid_public_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----"));
    }
    #[test]
    fn legacy_settings_and_nonsecret_server_draft_round_trip() {
        let mut config: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.server_setup.ssh_port,22);
        config.server_setup.host = "my-vps.example".into();
        config.server_setup.ssh_user = "ubuntu".into();
        let encoded=serde_json::to_string(&config).unwrap();
        assert!(encoded.contains("serverSetup"));
        assert!(encoded.contains("my-vps.example"));
        assert!(!encoded.contains("privateKey"));
        assert!(!encoded.contains("password"));
        assert_eq!(serde_json::from_str::<AppConfig>(&encoded).unwrap()
            .server_setup,config.server_setup);
        for host in ["http://bad.com","foo/bar","foo;id","-danger","evil host"] {
            assert!(!ServerBootstrapDraft::valid_host(host));
        }
        for user in ["root","root;id","-i","../../id","Ubuntu","user name"] {
            assert!(!ServerBootstrapDraft::valid_user(user));
        }
        assert!(!ServerBootstrapDraft::valid_fingerprint("MD5:some-key"));
        assert!(!ServerBootstrapDraft::valid_port(0));
    }
}
