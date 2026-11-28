use super::*;

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
    downloads::controls(&mut rows, view);
    if !view.loaded || view.preparing {
        rows.push(status("progress", label("preparing"), Tone::Secondary));
    }
    if let Some(notice) = &view.notice {
        rows.push(notice::row(notice));
    }
    rows
}
