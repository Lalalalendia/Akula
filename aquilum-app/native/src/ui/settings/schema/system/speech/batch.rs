use super::*;
use crate::workspace::speech_jobs::Notice;
use aquilum_core::background_jobs::speech::MAX_BATCH_FILES;

pub(super) fn controls(view: &View) -> Vec<Row> {
    let mut rows = vec![
        info(
            label("source"),
            Some(label("sourceHint")),
            buttons(vec![
                button("chooseMedia", Speech::PickMedia).enabled(view.ready()),
            ]),
        ),
        info(
            label("batch"),
            Some(label("batchHint")),
            buttons(vec![
                button("chooseBatch", Speech::PickBatch).enabled(view.ready()),
            ]),
        ),
    ];
    let (description, control) = if view.processing_queue {
        let description = view.active_source.as_deref().map(|source| {
            t_with(
                "settings.system.speech.queueActive",
                &[("name", &file_name(source))],
            )
        });
        (description, button("stopQueue", Speech::StopQueue))
    } else {
        (
            Some(label("queueHint")),
            button("runQueue", Speech::RunQueue).enabled(view.ready() && view.has_queued),
        )
    };
    rows.push(info(
        label("queueMode"),
        description,
        buttons(vec![control]),
    ));
    if !view.loaded || view.preparing {
        rows.push(status("progress", label("preparing"), Tone::Secondary));
    }
    if let Some(notice) = &view.notice {
        let (message, tone) = match notice {
            Notice::Error(error) => (error.chars().take(300).collect(), Tone::Danger),
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
        };
        rows.push(status("status", message, tone));
    }
    rows
}
