//! File transfer view models and feature-gated action data.
use super::*;

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

#[cfg(feature = "p2p-inbox-preview")]
#[derive(Clone, Debug)]
pub enum InboxAction {
    Preview(String),
    SaveCopy(String),
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
    pub previewing: bool,
    pub preview: Option<(String, String)>,
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
    pub inbox_save_actions: Vec<(WidgetId, InboxAction)>,
    #[cfg(feature = "p2p-native-send-beta")]
    pub native_send_actions: Vec<(WidgetId, NativeSendAction)>,
    #[cfg(feature = "p2p-native-receive-beta")]
    pub native_receive_actions: Vec<(WidgetId, NativeReceiveAction)>,
    #[cfg(feature = "messaging-w4-pilot")]
    pub w4_actions: Vec<(WidgetId, PilotAction)>,
}
