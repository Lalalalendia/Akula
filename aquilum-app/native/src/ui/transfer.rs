//! Transfer preparation UI. No transport or file authorization is exposed here.
//! Rows are in-memory drafts, never a claim that data was sent or verified.

use aquilum_core::sync::DeviceId;
use masonry::accesskit::Role;
use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::kurbo::{Axis, Size};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::dialog::{CardSize, Modal, header};
use super::scroll::ScrollArea;
use super::text::selectable;
use super::tokens::size;
use super::widgets::{IconButton, Leading, Rule, TextButton, Variant};
use super::{icons, theme};
use crate::i18n::{t, t_with};

#[derive(Clone, Debug)]
pub struct QueueRow {
    pub index: usize,
    pub name: String,
    pub bytes: u64,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueAction {
    Toggle,
    Remove,
}

mod devices;
pub use devices::{DeviceRow, DeviceStatus};
#[cfg(feature = "p2p-device-contacts-preview")]
mod contact_preview;
#[cfg(feature = "p2p-device-contacts-preview")]
pub use contact_preview::{TransportContactRow, TransportContactView};

pub struct TransferDialog {
    pub layer: WidgetId,
    pub close: WidgetId,
    pub choose: WidgetId,
    pub actions: Vec<(WidgetId, usize, QueueAction)>,
    pub device_actions: Vec<(WidgetId, DeviceId, [u8; 32])>,
}

fn card_size(room: Size) -> Size {
    Size::new(room.width.min(740.0), room.height.min(600.0))
}

fn rule() -> NewWidget<Rule> {
    Rule::new(Axis::Horizontal, || theme::current().border_default)
}

fn details(value: &str) -> NewWidget<super::widgets::Label> {
    selectable(value, |theme| theme.text_secondary, None, Leading::Px(size::SIZE_20 as f32))
}

fn hint(value: &str) -> NewWidget<super::widgets::Label> {
    selectable(value, |theme| theme.text_tertiary, None, Leading::Px(size::SIZE_20 as f32))
}

fn display_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

mod dialog_view;
pub use dialog_view::dialog;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_size_always_refers_to_source_metadata_not_network_progress() {
        assert_eq!(display_size(0), "0 B");
        assert_eq!(display_size(1024), "1.0 KiB");
        assert_eq!(display_size(1_048_576), "1.0 MiB");
    }
}
