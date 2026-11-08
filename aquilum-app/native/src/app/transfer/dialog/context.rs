//! Typed transfer dialog snapshot, keeping overlay/W4 layout separate.
use super::super::*;

impl TransferState {
    pub(super) fn dialog_extras(&self) -> transfer_ui::TransferExtras<'_> {
        let identity_view = {
            #[cfg(feature = "p2p-device-ui")]
            {
                Some(&self.local_identity.view)
            }
            #[cfg(not(feature = "p2p-device-ui"))]
            {
                None
            }
        };
        transfer_ui::TransferExtras {
            identity: identity_view,
            #[cfg(feature = "p2p-device-ui")]
            peer: if matches!(
                self.local_identity.view.phase,
                transfer_ui::IdentityPhase::Ready(_)
            ) {
                Some(&self.peer_pair.view)
            } else {
                None
            },
            #[cfg(feature = "p2p-inbox-preview")]
            inbox: Some(&self.inbox.view),
            #[cfg(feature = "p2p-native-send-beta")]
            sender: Some(&self.sender.view),
            #[cfg(feature = "p2p-native-send-beta")]
            can_send: self.drafts.len() == 1 && !self.drafts[0].paused,
            #[cfg(feature = "p2p-native-receive-beta")]
            receiver: Some(&self.receiver.view),
            #[cfg(feature = "messaging-w4-pilot")]
            w4: Some(&self.w4_pilot.view),
        }
    }
}
