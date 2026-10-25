#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerProbeView {
    Checking,
    Observed { facts: String },
    Unsupported { facts: String },
    NeedsAdminAuthorization { facts: String },
    Failed,
}
