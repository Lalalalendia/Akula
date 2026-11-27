use super::*;
use crate::workspace::speech_jobs::Notice;
use aquilum_core::background_jobs::speech::MAX_BATCH_FILES;
pub(super) fn row(notice: &Notice) -> Row {
    let (message, tone) = match notice {
        Notice::Error(error) => (error.chars().take(300).collect(), Tone::Danger),
        Notice::MemoryPressure => (label("memoryPressure"), Tone::Danger),
        Notice::LeaseBusy => (label("leaseBusy"), Tone::Secondary),
        Notice::Exported => (label("exported"), Tone::Secondary),
        Notice::BatchTooLarge => (
            t_with(
                "settings.system.speech.batchLimit",
                &[("limit", &MAX_BATCH_FILES.to_string())],
            ),
            Tone::Danger,
        ),
        Notice::Batch(report) => {
            let mut summary = t_with(
                "settings.system.speech.batchResult",
                &[
                    ("accepted", &report.accepted.to_string()),
                    ("rejected", &report.rejected.to_string()),
                ],
            );
            if let Some(example) = &report.example {
                summary.push_str("\n");
                summary.push_str(example);
            }
            let tone = if report.rejected == 0 {
                Tone::Secondary
            } else {
                Tone::Danger
            };
            (summary, tone)
        }
        Notice::DownloadQueued(path) => (
            t_with(
                "settings.system.speech.downloadQueued",
                &[("name", &file_name(path))],
            ),
            Tone::Secondary,
        ),
        Notice::DownloadError(path, error) => {
            let title = t_with(
                "settings.system.speech.downloadError",
                &[("name", &file_name(path))],
            );
            (
                format!("{title}\n{}", error.chars().take(200).collect::<String>()),
                Tone::Danger,
            )
        }
    };
    status("status", message, tone)
}
