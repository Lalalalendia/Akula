//! Read-only SSH status: observations never grant installation.
use super::*;
pub(super) fn status_row(probe: Option<&ServerProbeView>) -> Row {
    let state_key = match probe {
        Some(ServerProbeView::Checking) => "settings.server.probing",
        Some(ServerProbeView::Observed { .. }) => "settings.server.verifiedReadOnly",
        Some(ServerProbeView::Unsupported { .. }) => "settings.server.unsupported",
        Some(ServerProbeView::Failed) => "settings.server.verificationFailed",
        None => "settings.server.notConnected",
    };
    info(t("settings.server.state"), None,
        Control::Status {
            text: t(state_key),
            tone: match probe {
                Some(ServerProbeView::Failed | ServerProbeView::Unsupported { .. }) => Tone::Danger,
                Some(ServerProbeView::Observed { .. }) => Tone::Secondary,
                _ => Tone::Muted,
            },
        },
    )
}
pub(super) fn append_facts(rows: &mut Vec<Row>, probe: Option<&ServerProbeView>) {
    if let Some(ServerProbeView::Observed { facts }
        | ServerProbeView::Unsupported { facts }) = probe
    {
        // Only normalized, bounded typed host facts; never SSH stderr,
        // shell output, credentials or an installation-success claim.
        rows.push(info(
            t("settings.server.observedFacts"),
            Some(t("settings.server.observedFactsHint")),
            Control::Status {
                text: facts.clone(),
                tone: Tone::Secondary,
            },
        ));
    }
}
