use super::*;
use aquilum_core::background_jobs::{Queue, Segment, Spec, speech};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

fn publish(h: &Harness, bytes: &[u8]) -> std::path::PathBuf {
    let path = h.vault.path().join("recording.webm");
    std::fs::write(&path, bytes).unwrap();
    let path = path.canonicalize().unwrap();
    let revision: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut queue = Queue::open(&h.app.workspace.data_dir.join("media-jobs.sqlite")).unwrap();
    let id = queue
        .enqueue(&Spec {
            kind: speech::KIND.into(),
            revision,
            source: path.to_str().unwrap().into(),
            recipe: "test".into(),
        })
        .unwrap();
    let lease = queue
        .claim(speech::KIND, Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    queue.set_total(&lease, 1).unwrap();
    queue
        .append(
            &lease,
            &Segment {
                ordinal: 0,
                start_ms: 0,
                end_ms: 4_000,
                payload: serde_json::to_vec(&[speech::Cue {
                    start_ms: 2_325,
                    end_ms: 3_700,
                    text: "old cue".into(),
                }])
                .unwrap(),
            },
        )
        .unwrap();
    queue.finish(&lease).unwrap();
    speech::edits::save(
        &queue,
        id,
        speech::edits::CueId {
            window: 0,
            index: 0,
        },
        0,
        "zephyrlocator",
    )
    .unwrap();
    path
}

fn open(h: &mut Harness, path: &std::path::Path) {
    h.app.open_search(&mut h.root);
    h.type_search("zephyrlocator");
    assert_eq!(h.wait_results(1), 1);
    assert!(matches!(&h.app.popup, Some(Popup::Search(state)) if state.selected_path() == path));
    h.app.search_key(
        &mut h.root,
        &Key::Named(NamedKey::Enter),
        Modifiers::empty(),
    );
    assert!(matches!(&h.app.popup, Some(Popup::Transcript(_))));
}

#[test]
fn search_opens_edited_speech_in_the_transcript_editor() {
    let mut h = Harness::new();
    let path = publish(&h, b"fixture revision");
    open(&mut h, &path);
    assert!(wait_until(250, 20, || {
        h.app.tick(&mut h.root, Instant::now());
        matches!(&h.app.popup, Some(Popup::Transcript(editor))
            if editor.page_for_test().is_some_and(|page| page.rows[0].cue.start_ms == 2325))
    }));
}

#[cfg(all(target_os = "linux", feature = "memory-probe"))]
#[test]
#[ignore = "requires the local four-second VP8 video fixture"]
fn real_speech_search_seeks_to_the_result() {
    let mut h = Harness::new();
    let bytes = std::fs::read(std::env::var_os("AKULA_ASR_CAPTION_VIDEO").unwrap()).unwrap();
    let path = publish(&h, &bytes);
    open(&mut h, &path);
    assert!(wait_until(250, 20, || {
        h.app.tick(&mut h.root, Instant::now());
        h.anim(Duration::from_millis(16));
        matches!(&h.app.popup, Some(Popup::Transcript(editor))
            if editor.playback_for_test().0.is_some() && (editor.playback_for_test().1 - 2.325).abs() <= 0.12)
    }));
}
