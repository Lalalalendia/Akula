mod playback;
use super::*;
use crate::ui::settings::{Binding, schema::Action};
use crate::workspace::speech_jobs::{Action as Speech, Command, Tool, View};
use aquilum_core::background_jobs::{Queue, Spec, State, speech};
use std::path::PathBuf;
use std::time::{Duration, Instant};
fn wait(h: &mut Harness, condition: impl Fn(&View) -> bool) -> View {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.app.tick(&mut h.root, Instant::now());
        let view = h.app.workspace.speech().view();
        if condition(&view) {
            h.app.tick(&mut h.root, Instant::now());
            return view;
        }
        assert!(Instant::now() < deadline, "speech view: {view:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn press(h: &mut Harness, action: Speech) {
    let target = h.binding(
        "speech action in System settings",
        |binding| matches!(binding, Binding::Button(Action::Speech(item)) if item == &action),
    );
    h.send(target, crate::ui::widgets::Pressed);
}

#[test]
fn speech_dispatcher_cancels_persisted_jobs_from_system_settings() {
    let mut h = Harness::new();
    let database = h.app.workspace.data_dir.join("media-jobs.sqlite");
    let queue = Queue::open(&database).unwrap();
    let id = queue
        .enqueue(&Spec {
            kind: speech::KIND.into(),
            source: "local.mp4".into(),
            revision: "2".repeat(64),
            recipe: "test".into(),
        })
        .unwrap();
    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
    wait(&mut h, |view| view.jobs.iter().any(|job| job.id == id));
    press(&mut h, Speech::Cancel(id));
    wait(&mut h, |view| {
        view.jobs
            .iter()
            .any(|job| job.id == id && job.state == State::Cancelled)
    });
    assert_eq!(
        Queue::open(&database)
            .unwrap()
            .get(id)
            .unwrap()
            .unwrap()
            .state,
        State::Cancelled
    );
}

#[test]
#[ignore = "requires explicitly supplied FFprobe, FFmpeg, whisper-cli, models and a spoken WAV"]
fn real_speech_job_from_settings_can_export_captions() {
    let argument =
        |key| PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("missing {key}")));
    let mut h = Harness::new();
    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
    let keys = [
        "AKULA_ASR_PROBE",
        "AKULA_ASR_FFMPEG",
        "AKULA_ASR_ENGINE",
        "AKULA_ASR_MODEL",
        "AKULA_ASR_VAD",
    ];
    for (tool, key) in Tool::ALL.into_iter().zip(keys) {
        h.app
            .workspace
            .speech()
            .send(Command::SetTool(tool, argument(key)));
    }
    wait(&mut h, View::ready);
    h.app
        .workspace
        .speech()
        .send(Command::Inspect(argument("AKULA_ASR_SAMPLE")));
    let ready = wait(&mut h, |view| view.selection.is_some());
    let track = ready.selection.unwrap().tracks[0].index;
    press(&mut h, Speech::UseTrack(track));
    let queued = wait(&mut h, |view| !view.jobs.is_empty());
    let id = queued.jobs[0].id;
    press(&mut h, Speech::Run(id));
    wait(&mut h, |view| view.active == Some(id));
    press(&mut h, Speech::Pause(id));
    wait(&mut h, |view| {
        view.active.is_none()
            && view
                .jobs
                .iter()
                .any(|job| job.id == id && job.state == State::Paused)
    });
    press(&mut h, Speech::Run(id));
    wait(&mut h, |view| {
        view.jobs
            .iter()
            .any(|job| job.id == id && job.state == State::Completed)
    });
    let target = h.app.workspace.data_dir.join("captions.vtt");
    h.app
        .workspace
        .speech()
        .send(Command::Export(id, target.clone()));
    wait(&mut h, |view| {
        view.notice == Some(crate::workspace::speech_jobs::Notice::Exported)
    });
    let vtt = std::fs::read_to_string(target).unwrap();
    assert!(
        vtt.starts_with("WEBVTT\n") && vtt.contains(" --> "),
        "{vtt}"
    );
}
