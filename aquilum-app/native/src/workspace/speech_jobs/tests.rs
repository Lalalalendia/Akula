mod batch;
use super::*;
use aquilum_core::background_jobs::{Queue, Segment, Spec, speech};
use std::path::Path;
use std::time::Instant;

fn open(profile: &Path) -> Queue {
    Queue::open(&profile.join("media-jobs.sqlite")).unwrap()
}

fn seed(queue: &Queue, number: usize) -> JobId {
    queue
        .enqueue(&Spec {
            kind: speech::KIND.into(),
            source: format!("clip-{number}.mp4"),
            revision: format!("{number:064x}"),
            recipe: "test".into(),
        })
        .unwrap()
}

fn until(dispatcher: &Dispatcher, accepts: impl Fn(&View) -> bool) -> View {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let state = dispatcher.view();
        if accepts(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "latest dispatcher state: {state:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn pages_queue_and_persists_control_transitions_with_committed_speech() {
    let temp = tempfile::tempdir().unwrap();
    let mut queue = open(temp.path());
    for number in 1..=10 {
        seed(&queue, number);
    }
    let id = seed(&queue, 5);
    let lease = queue
        .claim(speech::KIND, Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    queue.set_total(&lease, 2).unwrap();
    queue
        .append(
            &lease,
            &Segment {
                ordinal: 0,
                start_ms: 0,
                end_ms: 30_000,
                payload: b"[]".to_vec(),
            },
        )
        .unwrap();

    let dispatcher = Dispatcher::new(temp.path().into(), Arc::new(|| {}));
    let first = until(&dispatcher, |view| view.loaded);
    assert_eq!(first.jobs.len(), PAGE_SIZE as usize);
    assert!(first.older && !first.newer);
    dispatcher.send(Command::Action(Action::Older));
    assert_eq!(until(&dispatcher, |view| view.newer).jobs.len(), 2);
    dispatcher.send(Command::Action(Action::Newer));
    until(&dispatcher, |view| !view.newer);
    dispatcher.send(Command::Action(Action::Pause(id)));
    let is_state = |view: &View, state| {
        view.jobs
            .iter()
            .any(|job| job.id == id && job.state == state && job.committed == 1)
    };
    until(&dispatcher, |view| is_state(view, State::Paused));
    dispatcher.send(Command::Action(Action::Cancel(id)));
    until(&dispatcher, |view| is_state(view, State::Cancelled));
    drop(dispatcher);

    let dispatcher = Dispatcher::new(temp.path().into(), Arc::new(|| {}));
    until(&dispatcher, |view| is_state(view, State::Cancelled));
    assert_eq!(queue.segments(id, 0, 1).unwrap()[0].payload, b"[]");
}

#[test]
fn tool_selection_survives_reopen_and_refuses_a_changed_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("selected-tool");
    std::fs::write(&path, b"first").unwrap();
    let mut config = config::Config::default();
    config
        .choose(temp.path(), Tool::Whisper, path.clone())
        .unwrap();
    let config = config::Config::load(temp.path()).unwrap();
    assert_eq!(
        config.checked(Tool::Whisper).unwrap(),
        path.canonicalize().unwrap()
    );
    std::fs::write(path, b"other").unwrap();
    assert!(config.checked(Tool::Whisper).is_err());
}

#[test]
fn export_does_not_replace_existing_file() {
    let temp = tempfile::tempdir().unwrap();
    let queue = open(temp.path());
    let id = seed(&queue, 1);
    let destination = temp.path().join("output.vtt");
    export::save(&queue, id, &destination).unwrap();
    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "WEBVTT\n\n");
    std::fs::write(&destination, "keep these bytes").unwrap();
    assert!(export::save(&queue, id, &destination).is_err());
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "keep these bytes"
    );
    assert!(!temp.path().read_dir().unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}
