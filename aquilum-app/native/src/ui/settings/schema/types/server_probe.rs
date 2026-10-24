/// Ephemeral SSH verification result, not a Node install receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerProbeView {
    Checking,
    Observed { facts: String },
    Unsupported { facts: String },
    Failed,
}
