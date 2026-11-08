//! Feature-gated Inbox actions and read-only UI projection.
#[cfg(feature = "p2p-inbox-preview")]
#[derive(Clone, Debug)]
pub enum InboxAction {
    Preview(String),
    SaveCopy(String),
    #[cfg(windows)]
    Import(String),
    AskRemoval(String),
    ConfirmRemoval(String),
    CancelRemoval,
}

#[derive(Clone, Debug)]
pub struct InboxRow {
    pub id: String,
    pub bytes: u64,
    pub sender: String,
    pub status_key: &'static str,
    pub import_audit_key: Option<&'static str>,
}

#[derive(Clone, Debug, Default)]
pub struct InboxView {
    pub rows: Vec<InboxRow>,
    pub total: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub exporting: bool,
    pub importing: bool,
    pub previewing: bool,
    pub removing: bool,
    pub removal_confirmation: Option<String>,
    pub preview: Option<(String, String)>,
    pub notice_key: Option<&'static str>,
}
