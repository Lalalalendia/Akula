use serde::{Deserialize, Serialize};
use super::FontSettings;

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
    pub paste_formatting: bool,
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
            paste_formatting: true,
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
