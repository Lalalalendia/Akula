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
