use super::{Binding, CloudPopup, Section};
use masonry::core::WidgetId;
use masonry::widgets::Flex;
use crate::ui::dialog::DialogIds;
use crate::ui::handle::Handle;
use crate::ui::scroll::ScrollArea;

pub struct SettingsDialog {
    pub layer: WidgetId,
    pub close: WidgetId,
    pub content: Handle<ScrollArea<Flex>>,
    pub nav: Vec<(WidgetId, Section)>,
    pub section: Section,
    pub bindings: HashMap<WidgetId, Binding>,
    pub copied: Option<usize>,
    pub trash_page: usize,
    pub sync_selected: Option<aquilum_core::sync::NativeRecoveryCasePreview>,
    pub sync_notice: Option<String>,
    pub server_advanced: bool,
    pub yandex_notice: Option<String>,
    pub sync_confirm: Option<(DialogIds, aquilum_core::sync::NativeRecoveryDecision)>,
    pub cloud_popup: Option<CloudPopup>,
}
