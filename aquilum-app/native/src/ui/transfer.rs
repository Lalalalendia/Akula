//! Transfer preparation UI. No transport or file authorization is exposed here.
//! Rows are in-memory drafts, never a claim that data was sent or verified.

use aquilum_core::sync::DeviceId;
use masonry::accesskit::Role;
use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::dialog::{CardSize, Modal, header};
use super::scroll::ScrollArea;
use super::tokens::size;
use super::widgets::{IconButton, TextButton, Variant};
use super::icons;
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

#[cfg(feature = "p2p-native-send-beta")]
#[derive(Clone, Debug)]
pub enum NativeSendAction {
    Start(String),
    StartInternet(String),
    Cancel,
}

#[cfg(feature = "p2p-native-receive-beta")]
#[derive(Clone, Debug)]
pub enum NativeReceiveAction {
    Start(String),
    StartInternet(String),
    Cancel,
}

/// Read-only view model; only the opt-in receiver-side worker populates rows.
/// Display labels never become filesystem paths.
#[derive(Clone, Debug)]
pub struct InboxRow {
    pub id: String,
    pub bytes: u64,
    pub sender: String,
    pub status_key: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct InboxView {
    pub rows: Vec<InboxRow>,
    pub total: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub exporting: bool,
    pub notice_key: Option<&'static str>,
}

#[cfg(feature = "p2p-native-send-beta")]
#[derive(Clone, Debug, Default)]
pub struct NativeSendView {
    pub busy: bool,
    pub canceling: bool,
    pub listening: Option<String>,
    pub message: Option<String>,
    pub success: bool,
}

#[cfg(feature = "p2p-native-receive-beta")]
#[derive(Clone, Debug, Default)]
pub struct NativeReceiveView {
    pub busy: bool,
    pub canceling: bool,
    pub success: bool,
    pub message: Option<String>,
}

mod devices;
mod identity;
#[cfg(feature = "p2p-device-ui")]
mod peer_pair;
mod layout;
mod queue_view;
#[cfg(feature = "p2p-inbox-preview")]
mod inbox_view;
#[cfg(feature = "p2p-native-send-beta")]
mod native_send_view;
#[cfg(feature = "p2p-native-receive-beta")]
mod native_receive_view;
use layout::{card_size, details, display_size, hint, rule};
pub use devices::{DeviceRow, DeviceStatus};
pub use identity::{IdentityAction, IdentityPhase, IdentityView};
#[cfg(feature = "p2p-device-ui")]
pub use peer_pair::{PeerAction, PeerControls, PeerPhase, PeerView};
#[cfg(feature = "p2p-device-contacts-preview")]
mod contact_preview;
#[cfg(feature = "p2p-device-contacts-preview")]
pub use contact_preview::{TransportContactRow, TransportContactView};

/// One typed, feature-gated bundle avoids passing different positional
/// arguments from every independent P2P/W4 feature combination.
#[derive(Default)]
pub struct TransferExtras<'a> {
    pub identity: Option<&'a IdentityView>,
    #[cfg(feature = "p2p-device-ui")]
    pub peer: Option<&'a PeerView>,
    #[cfg(feature = "p2p-inbox-preview")]
    pub inbox: Option<&'a InboxView>,
    #[cfg(feature = "p2p-native-send-beta")]
    pub sender: Option<&'a NativeSendView>,
    #[cfg(feature = "p2p-native-send-beta")]
    pub can_send: bool,
    #[cfg(feature = "p2p-native-receive-beta")]
    pub receiver: Option<&'a NativeReceiveView>,
    #[cfg(feature = "messaging-w4-pilot")]
    pub w4: Option<&'a PilotView>,
}

pub struct TransferDialog {
    pub layer: WidgetId,
    pub close: WidgetId,
    pub choose: WidgetId,
    pub actions: Vec<(WidgetId, usize, QueueAction)>,
    pub device_actions: Vec<(WidgetId, DeviceId, [u8; 32])>,
    pub identity_actions: Vec<(WidgetId, IdentityAction)>,
    #[cfg(feature = "p2p-device-ui")]
    pub peer_controls: Option<PeerControls>,
    #[cfg(feature = "p2p-inbox-preview")]
    pub inbox_save_actions: Vec<(WidgetId, String)>,
    #[cfg(feature = "p2p-native-send-beta")]
    pub native_send_actions: Vec<(WidgetId, NativeSendAction)>,
    #[cfg(feature = "p2p-native-receive-beta")]
    pub native_receive_actions: Vec<(WidgetId, NativeReceiveAction)>,
    #[cfg(feature = "messaging-w4-pilot")]
    pub w4_actions: Vec<(WidgetId, PilotAction)>,
}

#[cfg(feature = "messaging-w4-pilot")]
mod w4_pilot;
#[cfg(feature = "messaging-w4-pilot")]
pub use w4_pilot::{PilotAction, PilotView};
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
