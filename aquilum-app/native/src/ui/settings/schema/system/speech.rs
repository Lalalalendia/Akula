mod item;
use super::*;
use crate::workspace::speech_jobs::{Action as Speech, Notice, Tool, View};
use aquilum_core::background_jobs::State;
use item::{button, buttons, file_name, label, status};
use std::path::Path;

pub(super) fn block(view: &View) -> Block {
    let mut rows = vec![info(
        label("source"),
        Some(label("sourceHint")),
        buttons(vec![
            button("chooseMedia", Speech::PickMedia).enabled(view.ready()),
        ]),
    )];
    if !view.loaded || view.preparing {
        rows.push(status("progress", label("preparing"), Tone::Secondary));
    }
    if let Some(notice) = &view.notice {
        let (message, tone) = match notice {
            Notice::Error(error) => (error.chars().take(300).collect(), Tone::Danger),
            Notice::LeaseBusy => (label("leaseBusy"), Tone::Secondary),
            Notice::Exported => (label("exported"), Tone::Secondary),
        };
        rows.push(status("status", message, tone));
    }
    if let Some(selection) = &view.selection {
        rows.push(info(
            file_name(&selection.source),
            Some(label("selectTrack")),
            buttons(vec![button("dismiss", Speech::DismissMedia)]),
        ));
        for track in &selection.tracks {
            let desc = [track.language.as_deref(), track.title.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            rows.push(info(
                t_with(
                    "settings.system.speech.track",
                    &[("index", &track.index.to_string())],
                ),
                (!desc.is_empty()).then(|| desc.chars().take(160).collect()),
                buttons(vec![
                    button("enqueue", Speech::UseTrack(track.index)).enabled(view.ready()),
                ]),
            ));
        }
    }
    if view.jobs.is_empty() && view.loaded {
        rows.push(status("queue", label("empty"), Tone::Muted));
    }
    for job in &view.jobs {
        let active = view.active == Some(job.id);
        let state = label(match job.state {
            State::Queued => "queued",
            State::Running if active => "running",
            State::Running => "awaitingRecovery",
            State::Paused => "paused",
            State::Cancelled => "cancelled",
            State::Failed => "failed",
            State::Completed => "completed",
        });
        let mut description = match job.total {
            Some(total) => format!("{state} · {}/{}", job.committed, total),
            None => state,
        };
        if let Some(error) = &job.error {
            description.push_str(" · ");
            description.extend(error.chars().take(240));
        }
        let mut actions = match job.state {
            State::Completed | State::Cancelled => Vec::new(),
            State::Running | State::Queued if active => {
                vec![
                    button("pause", Speech::Pause(job.id)),
                    button("cancel", Speech::Cancel(job.id)),
                ]
            }
            state => vec![
                button(
                    if state == State::Queued {
                        "start"
                    } else {
                        "resume"
                    },
                    Speech::Run(job.id),
                )
                .enabled(view.ready()),
                button("cancel", Speech::Cancel(job.id)),
            ],
        };
        if job.committed > 0 {
            actions.push(
                button("showCaptions", Speech::ShowCaptions(job.id)).enabled(
                    view.caption_job != Some(job.id) && view.active.is_none() && !view.preparing,
                ),
            );
            actions.push(
                button("export", Speech::Export(job.id))
                    .enabled(view.active.is_none() && !view.preparing),
            );
        }
        rows.push(info(
            file_name(&job.source),
            Some(description),
            buttons(actions),
        ));
    }
    if view.caption_job.is_some() {
        rows.push(info(
            label("captionsEnabled"),
            None,
            buttons(vec![button("hideCaptions", Speech::HideCaptions)]),
        ));
    }
    rows.push(info(
        label("pages"),
        None,
        buttons(vec![
            button("newer", Speech::Newer).enabled(view.newer),
            button("older", Speech::Older).enabled(view.older),
            button("refresh", Speech::Refresh),
        ]),
    ));
    rows.push(status("tools", label("toolsHint"), Tone::Muted));
    for tool in Tool::ALL {
        let file = view.tools[tool as usize].as_deref();
        let name = match tool {
            Tool::Model => label("model"),
            Tool::Vad => label("vad"),
            _ => tool.label().to_owned(),
        };
        rows.push(info(
            name,
            Some(file.map_or_else(|| label("missing"), file_name)),
            buttons(vec![button("chooseTool", Speech::PickTool(tool)).enabled(
                view.loaded && view.active.is_none() && !view.preparing,
            )]),
        ));
    }
    Block {
        title: label("title"),
        rows,
    }
}
