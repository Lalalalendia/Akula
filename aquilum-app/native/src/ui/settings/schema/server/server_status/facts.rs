//! Read-only bounded host-fact rendering; never grants install authority.
use super::*;
pub(crate) fn append_facts(rows: &mut Vec<Row>, probe: Option<&ServerProbeView>) {
    if let Some(ServerProbeView::Observed { facts }
        | ServerProbeView::Unsupported { facts }
        | ServerProbeView::NeedsAdminAuthorization { facts }) = probe
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
