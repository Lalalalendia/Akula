#![cfg(all(target_os = "linux", feature = "memory-probe"))]

use super::*;

#[test]
#[ignore = "requires GStreamer and a four-second VP8 clip with frequent keyframes"]
fn real_playback_captions_follow_forward_and_backward_seek() {
    let mut h = Harness::new();
    let fixture = PathBuf::from(std::env::var_os("AKULA_ASR_CAPTION_VIDEO").unwrap());
    let path = h.vault.path().join("caption-player.webm");
    std::fs::copy(fixture, &path).unwrap();
    let path = path.canonicalize().unwrap();
    let digest = std::env::var("AKULA_ASR_CAPTION_SHA256").unwrap();
    let mut queue = Queue::open(&h.app.workspace.data_dir.join("media-jobs.sqlite")).unwrap();
    let id = queue
        .enqueue(&Spec {
            kind: speech::KIND.into(),
            source: path.to_str().unwrap().into(),
            revision: digest,
            recipe: "test".into(),
        })
        .unwrap();
    let lease = queue
        .claim(speech::KIND, Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    queue.set_total(&lease, 1).unwrap();
    let cues = [
        speech::Cue {
            start_ms: 250,
            end_ms: 1_250,
            text: "First caption".into(),
        },
        speech::Cue {
            start_ms: 2_000,
            end_ms: 3_750,
            text: "Second caption".into(),
        },
    ];
    queue
        .append(
            &lease,
            &aquilum_core::background_jobs::Segment {
                ordinal: 0,
                start_ms: 0,
                end_ms: 4_000,
                payload: serde_json::to_vec(&cues).unwrap(),
            },
        )
        .unwrap();
    queue.finish(&lease).unwrap();

    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
    wait(&mut h, |view| view.jobs.iter().any(|job| job.id == id));
    press(&mut h, Speech::ShowCaptions(id));
    wait(&mut h, |view| view.caption_job == Some(id));

    let close = match h.app.popup.as_ref().unwrap() {
        Popup::Settings { dialog, .. } => dialog.close,
        _ => panic!("settings are not open"),
    };
    h.send(close, crate::ui::widgets::Pressed);
    assert!(h.app.popup.is_none());
    h.settle();
    h.deliver();
    let note = h.write_note("captions.md", "![[caption-player.webm]]");
    h.app.open(&mut h.root, note);
    let deadline = Instant::now() + Duration::from_secs(6);
    while crate::video::state(&path).is_none() {
        h.step();
        let body = h.app.note_ids.as_ref().unwrap().body;
        body.edit(&mut h.root, |mut editor| {
            crate::ui::editor::TextEditor::probe_play(&mut editor)
        });
        assert!(Instant::now() < deadline, "embedded player did not start");
        std::thread::sleep(Duration::from_millis(16));
    }
    let mut owned = crate::video::Owned::default();
    owned.add(path.clone());
    let seek_until = |h: &mut Harness,
                      expected: Option<&str>,
                      seek: Option<f64>,
                      owned: &crate::video::Owned| {
        if let Some(fraction) = seek {
            crate::video::seek(&path, fraction);
        }
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            owned.tick();
            h.app.tick(&mut h.root, Instant::now());
            let state = crate::video::state(&path).unwrap();
            let text = crate::video::captions::text_at(&path, state.position);
            let in_gap = expected.is_some() || (1.25..2.0).contains(&state.position);
            if state.frame.is_some() && in_gap && text.as_deref() == expected {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "position {} captions {text:?}",
                state.position
            );
            std::thread::sleep(Duration::from_millis(16));
        }
    };
    seek_until(&mut h, Some("First caption"), None, &owned);
    crate::video::toggle(&path);
    seek_until(&mut h, Some("Second caption"), Some(0.65), &owned);
    seek_until(&mut h, None, Some(0.40), &owned);
    seek_until(&mut h, Some("First caption"), Some(0.20), &owned);
    let editor = h.app.editor_id().unwrap();
    h.root
        .edit_widget(editor, |mut editor| editor.ctx.request_post_paint());
    h.snapshot("asr08-paused-after-backward-seek.png");
    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
    press(&mut h, Speech::HideCaptions);
    wait(&mut h, |view| view.caption_job.is_none());
    assert_eq!(crate::video::captions::text_at(&path, 0.8), None);
}
