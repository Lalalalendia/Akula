//! Read-only SSH status: observations never grant installation.
use super::*;
pub(super) fn status_row(probe: Option<&ServerProbeView>) -> Row {
    let state_key = match probe {
        Some(ServerProbeView::Checking) => "settings.server.probing",
        Some(ServerProbeView::Observed { .. }) => "settings.server.verifiedReadOnly",
        Some(ServerProbeView::Unsupported { .. }) => "settings.server.unsupported",
        Some(ServerProbeView::NeedsAdminAuthorization { .. }) =>
            "settings.server.adminAuthorizationRequired",
        Some(ServerProbeView::Failed) => "settings.server.verificationFailed",
        None => "settings.server.notConnected",
    };
    info(t("settings.server.state"), None,
        Control::Status {
            text: t(state_key),
            tone: match probe {
                Some(ServerProbeView::Failed | ServerProbeView::Unsupported { .. }
                    | ServerProbeView::NeedsAdminAuthorization { .. }) => Tone::Danger,
                Some(ServerProbeView::Observed { .. }) => Tone::Secondary,
                _ => Tone::Muted,
            },
        },
    )
}
mod facts;
pub(super) use facts::append_facts;
