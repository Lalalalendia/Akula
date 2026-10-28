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
use super::icons;
use super::scroll::ScrollArea;
use super::tokens::size;
use super::widgets::{IconButton, TextButton, Variant};
use crate::i18n::{t, t_with};

mod types;
#[cfg(feature = "p2p-inbox-preview")]
pub use types::InboxAction;
pub use types::{InboxRow, InboxView, QueueAction, QueueRow, TransferDialog, TransferExtras};
#[cfg(feature = "p2p-native-receive-beta")]
pub use types::{NativeReceiveAction, NativeReceiveView};
#[cfg(feature = "p2p-native-send-beta")]
pub use types::{NativeSendAction, NativeSendView};

mod devices;
mod identity;
#[cfg(feature = "p2p-inbox-preview")]
mod inbox_view;
mod layout;
#[cfg(feature = "p2p-native-receive-beta")]
mod native_receive_view;
#[cfg(feature = "p2p-native-send-beta")]
mod native_send_view;
#[cfg(feature = "p2p-device-ui")]
mod peer_pair;
mod queue_view;
pub use devices::{DeviceRow, DeviceStatus};
pub use identity::{IdentityAction, IdentityPhase, IdentityView};
use layout::{card_size, details, display_size, hint, rule};
#[cfg(feature = "p2p-device-ui")]
pub use peer_pair::{PeerAction, PeerControls, PeerPhase, PeerView};
#[cfg(feature = "p2p-device-contacts-preview")]
mod contact_preview;
#[cfg(feature = "p2p-device-contacts-preview")]
pub use contact_preview::{TransportContactRow, TransportContactView};

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
