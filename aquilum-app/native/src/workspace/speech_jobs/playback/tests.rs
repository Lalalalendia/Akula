use super::super::{Action, Dispatcher, POLL};
use super::*;
use aquilum_core::background_jobs::{Segment, Spec, speech};
use std::time::{Duration, Instant};

fn await_condition(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !condition() {
        assert!(Instant::now() < deadline, "caption dispatcher timed out");
        std::thread::sleep(POLL / 10);
    }
}

#[test]
fn actor_pages_across_seeks_and_rejects_another_media_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.wav");
    std::fs::write(&path, b"abc").unwrap();
    let source = MediaSource::open(&path).unwrap();
    let mut queue = Queue::open(&dir.path().join("media-jobs.sqlite")).unwrap();
    let id = queue
        .enqueue(&Spec {
            kind: speech::KIND.into(),
            source: path.canonicalize().unwrap().to_str().unwrap().into(),
            revision: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            recipe: "test".into(),
        })
        .unwrap();
    let lease = queue
        .claim("speech", Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    queue.set_total(&lease, 6).unwrap();
    let append = |queue: &mut Queue, ordinal: i64| {
        let start = ordinal * 30_000;
        let cues = [speech::Cue {
            start_ms: start + 1_000,
            end_ms: start + 2_000,
            text: ordinal.to_string(),
        }];
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
    };
    for ordinal in 0..5 {
        append(&mut queue, ordinal);
    }

    let service = Dispatcher::new(dir.path().to_owned(), Arc::new(|| {}));
    service.send(Command::Action(Action::ShowCaptions(id)));
    let mut selected = None;
    await_condition(|| {
        selected = service.take_caption_update().flatten();
        selected.is_some()
    });
    let selected = selected.unwrap();
    await_condition(|| selected.text_at(&source, 1.0).as_deref() == Some("0"));
    await_condition(|| selected.text_at(&source, 121.1).as_deref() == Some("4"));
    assert_eq!(selected.text_at(&source, 151.7), None);
    append(&mut queue, 5);
    queue.finish(&lease).unwrap();
    service.send(Command::Action(Action::Refresh));
    await_condition(|| selected.text_at(&source, 151.7).as_deref() == Some("5"));
    await_condition(|| selected.text_at(&source, 1.9).as_deref() == Some("0"));
    assert_eq!(selected.text_at(&source, 2.0), None);
    assert_eq!(selected.text_at(&source, f64::NAN), None);
    let unrelated = dir.path().join("other.wav");
    std::fs::write(&unrelated, b"abc").unwrap();
    assert_eq!(
        selected.text_at(&MediaSource::open(&unrelated).unwrap(), 1.2),
        None
    );
    std::fs::write(&path, b"replacement").unwrap();
    assert_eq!(
        selected.text_at(&MediaSource::open(&path).unwrap(), 1.2),
        None
    );
    assert!(Transcript::verify(&queue, id).is_err());
    service.send(Command::Action(Action::HideCaptions));
    await_condition(|| service.view().caption_job.is_none());
}
