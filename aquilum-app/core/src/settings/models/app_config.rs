use super::*;

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
    pub server_setup: ServerBootstrapDraft,
    pub plugin_alpha: PluginAlphaSettings,
}

/// Host-backed local demonstration; not an arbitrary guest runtime or SDK.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginAlphaSettings {
    pub installed: bool,
    pub enabled: bool,
    /// Only the last count is persisted. No note contents are stored.
    pub last_words: Option<u64>,
    pub last_error: Option<String>,
}
