use super::*;
use crate::ui::dialog::DialogAnswer;
use crate::ui::editor::TextEditor;
use crate::ui::settings::transcript::{Button, Editor};
use crate::ui::widgets::Pressed;
use aquilum_core::background_jobs::{JobId, Segment, speech::edits};
use std::path::Path;

fn seed(h: &Harness, source: &Path, digest: String, windows: i64) -> (Queue, JobId) {
    let mut queue = Queue::open(&h.app.workspace.data_dir.join("media-jobs.sqlite")).unwrap();
    let id = queue
        .enqueue(&Spec {
            kind: speech::KIND.into(),
            source: source.to_str().unwrap().into(),
            revision: digest,
            recipe: "test".into(),
        })
        .unwrap();
    let lease = queue
        .claim(speech::KIND, Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    queue.set_total(&lease, windows).unwrap();
    for ordinal in 0..windows {
        let start = ordinal * 30_000;
        let cues = [
            speech::Cue {
                start_ms: start + 450,
                end_ms: start + 1_000,
                text: format!("Первая реплика {ordinal}"),
            },
            speech::Cue {
                start_ms: start + 2_325,
                end_ms: start + 3_750,
                text: "Вторая реплика".into(),
            },
        ];
        queue
            .append(
                &lease,
                &Segment {
                    ordinal,
                    start_ms: start,
                    end_ms: start + 30_000,
                    payload: serde_json::to_vec(&cues).unwrap(),
                },
            )
            .unwrap();
    }
    queue.finish(&lease).unwrap();
    (queue, id)
}

fn editor(h: &Harness) -> &Editor {
    let Some(Popup::Settings { dialog, .. }) = &h.app.popup else {
        panic!("settings closed");
    };
    dialog.transcript.as_ref().expect("transcript opened")
}

fn until(h: &mut Harness, predicate: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate(editor(h)) {
        h.app.tick(&mut h.root, Instant::now());
        h.anim(Duration::from_millis(16));
        assert!(
            Instant::now() < deadline,
            "transcript did not reach the requested state"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn click(h: &mut Harness, action: Button) {
    let id = editor(h).buttons_for_test(action).expect("button exists");
    h.act(id, Pressed);
}

fn open(h: &mut Harness, id: JobId) {
    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
    wait(h, |view| view.jobs.iter().any(|job| job.id == id));
    press(h, Speech::OpenTranscript(id));
    until(h, |editor| editor.page_for_test().is_some());
    h.settle();
}

#[test]
fn speech_transcript_pages_and_saves_from_settings() {
    let mut h = Harness::new();
    let (queue, id) = seed(&h, Path::new("video.mp4"), "2".repeat(64), 5);
    open(&mut h, id);
    click(&mut h, Button::Page(1));
    until(&mut h, |editor| editor.page_for_test().unwrap().number == 1);
    let row = editor(&h).page_for_test().unwrap().rows[0].clone();
    assert_eq!(row.id.window, 4);
    click(&mut h, Button::Edit(row.id));
    h.snapshot("asr-transcript-edit.png");

    let layer = editor(&h).layer();
    h.act(layer, DialogAnswer::Cancel);
    let input = editor(&h).input_for_test().expect("edit stays open");
    h.root
        .handle_text_event(masonry::core::TextEvent::WindowFocusChange(true));
    input.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, 0, row.cue.text.len());
    });
    h.type_text("Исправленная реплика");
    click(&mut h, Button::Save);
    until(&mut h, |editor| {
        editor.page_for_test().unwrap().rows[0].revision == 1
    });
    assert!(editor(&h).input_for_test().is_none());
    let mut vtt = Vec::new();
    speech::write_vtt(&queue, id, &mut vtt).unwrap();
    assert!(
        String::from_utf8(vtt)
            .unwrap()
            .contains("00:02:00.450 --> 00:02:01.000\nИсправленная реплика")
    );

    click(&mut h, Button::Close);
    assert!(!h.root.has_widget(layer));
    press(&mut h, Speech::OpenTranscript(id));
    until(&mut h, |editor| editor.page_for_test().is_some());
    click(&mut h, Button::Page(1));
    until(&mut h, |editor| editor.page_for_test().unwrap().number == 1);
    assert_eq!(
        editor(&h).page_for_test().unwrap().rows[0].cue.text,
        "Исправленная реплика"
    );
}

#[test]
fn speech_transcript_conflict_preserves_unsaved_text() {
    let mut h = Harness::new();
    let (queue, id) = seed(&h, Path::new("video.mp4"), "2".repeat(64), 1);
    open(&mut h, id);
    let row = editor(&h).page_for_test().unwrap().rows[0].clone();
    click(&mut h, Button::Edit(row.id));
    let input = editor(&h).input_for_test().unwrap();
    input.edit(&mut h.root, |mut editor| {
        TextEditor::reset_text(&mut editor, "Черновик")
    });
    edits::save(&queue, id, row.id, 0, "Параллельная правка").unwrap();

    click(&mut h, Button::Save);
    until(&mut h, |editor| editor.input_for_test().is_some());
    let draft = editor(&h)
        .input_for_test()
        .unwrap()
        .get(&h.root)
        .unwrap()
        .inner()
        .text();
    assert_eq!(draft, "Черновик");
    assert_eq!(
        edits::page(&queue, id, 0).unwrap().rows[0].cue.text,
        "Параллельная правка"
    );
}

#[cfg(all(target_os = "linux", feature = "memory-probe"))]
#[test]
#[ignore = "requires GStreamer and the four-second VP8 speech playback fixture"]
fn real_speech_transcript_timecodes_seek_the_preview_and_refresh_attached_captions() {
    let mut h = Harness::new();
    let path = h.vault.path().join("transcript.webm");
    std::fs::copy(std::env::var_os("AKULA_ASR_CAPTION_VIDEO").unwrap(), &path).unwrap();
    let path = path.canonicalize().unwrap();
    let digest = std::env::var("AKULA_ASR_CAPTION_SHA256").unwrap();
    let (_, id) = seed(&h, &path, digest, 1);
    open(&mut h, id);

    click(&mut h, Button::Jump(2_325));
    until(&mut h, |editor| {
        let (frame, position) = editor.playback_for_test();
        frame.is_some() && (position - 2.325).abs() < 0.12
    });
    h.snapshot("asr-transcript-playing.png");
    let first_frame = editor(&h).playback_for_test().0;
    click(&mut h, Button::Jump(450));
    until(&mut h, |editor| {
        let (frame, position) = editor.playback_for_test();
        frame.is_some() && frame != first_frame && (position - 0.45).abs() < 0.12
    });

    let dispatcher = h.app.workspace.speech();
    dispatcher.send(Command::Action(Speech::ShowCaptions(id)));
    let deadline = Instant::now() + Duration::from_secs(6);
    let track = loop {
        if let Some(Some(track)) = dispatcher.take_caption_update() {
            break track;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    let media = speech::playback::MediaSource::open(&path).unwrap();
    while track.text_at(&media, 0.45).is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let row = editor(&h).page_for_test().unwrap().rows[0].clone();
    click(&mut h, Button::Edit(row.id));
    let input = editor(&h).input_for_test().unwrap();
    input.edit(&mut h.root, |mut editor| {
        TextEditor::reset_text(&mut editor, "Субтитры после правки")
    });
    click(&mut h, Button::Save);
    until(&mut h, |editor| {
        editor.page_for_test().unwrap().rows[0].revision == 1
    });
    assert_eq!(
        track.text_at(&media, 0.45).as_deref(),
        Some("Субтитры после правки")
    );
}
