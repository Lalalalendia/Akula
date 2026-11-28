use super::*;
use std::sync::{Arc, Barrier};

const TTL: Duration = Duration::from_secs(60);

fn spec() -> Spec {
    Spec {
        kind: "speech".into(),
        source: "source-recording".into(),
        revision: "a".repeat(64),
        recipe: "model-v1:ru".into(),
    }
}

fn segment(ordinal: i64) -> Segment {
    Segment {
        ordinal,
        start_ms: ordinal * 1000,
        end_ms: (ordinal + 1) * 1000,
        payload: format!("segment {ordinal}").into_bytes(),
    }
}

#[test]
fn restart_preserves_checkpoint_and_deduplicates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("jobs.sqlite");
    let mut first = Queue::open(&path).unwrap();
    let id = first.enqueue(&spec()).unwrap();
    let lease = first.claim("speech", None, TTL).unwrap().unwrap();
    first.set_total(&lease, 2).unwrap();
    first.append(&lease, &segment(0)).unwrap();
    assert!(matches!(first.finish(&lease), Err(Error::Conflict)));
    first.pause(id).unwrap();
    drop(first);

    let mut second = Queue::open(&path).unwrap();
    assert_eq!(second.enqueue(&spec()).unwrap(), id);
    let saved = second.get(id).unwrap().unwrap();
    assert_eq!(
        (saved.state, saved.committed, saved.total),
        (State::Paused, 1, Some(2))
    );
    assert_eq!(second.segments(id, 0, 10).unwrap(), [segment(0)]);
    second.resume(id).unwrap();
    let resumed = second.claim("speech", Some(id), TTL).unwrap().unwrap();
    assert!(matches!(
        second.append(&lease, &segment(1)),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        second.append(&resumed, &segment(0)),
        Err(Error::Conflict)
    ));
    second.append(&resumed, &segment(1)).unwrap();
    second.finish(&resumed).unwrap();
    assert_eq!(second.get(id).unwrap().unwrap().state, State::Completed);
    assert_eq!(second.segments(id, 1, 1).unwrap(), [segment(1)]);
}

#[test]
fn expired_lease_takeover_cannot_lose_committed_output() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("jobs.sqlite");
    let mut old = Queue::open(&path).unwrap();
    let id = old.enqueue(&spec()).unwrap();
    let before = now().unwrap();
    let old_lease = old
        .claim_at("speech", Some(id), TTL, before)
        .unwrap()
        .unwrap();
    old.set_total(&old_lease, 2).unwrap();
    old.append(&old_lease, &segment(0)).unwrap();

    let mut new = Queue::open(&path).unwrap();
    assert!(new
        .claim_at("speech", Some(id), TTL, before + 59_999)
        .unwrap()
        .is_none());
    let new_lease = new
        .claim_at("speech", Some(id), TTL, before + 60_000)
        .unwrap()
        .unwrap();
    assert!(matches!(
        old.heartbeat(&old_lease, TTL),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        old.fail(&old_lease, "old failure"),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        old.append(&old_lease, &segment(1)),
        Err(Error::Conflict)
    ));
    assert_eq!(new.get(id).unwrap().unwrap().committed, 1);
    new.append(&new_lease, &segment(1)).unwrap();
    new.finish(&new_lease).unwrap();
    assert_eq!(new.segments(id, 0, 100).unwrap(), [segment(0), segment(1)]);
}

#[test]
fn concurrent_writers_get_one_job_and_one_lease() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("jobs.sqlite");
    let start = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                let queue = Queue::open(&path).unwrap();
                let id = queue.enqueue(&spec()).unwrap();
                let lease = queue.claim("speech", None, TTL).unwrap();
                (id, lease.is_some())
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|child| child.join().unwrap())
        .collect();
    assert!(results.iter().all(|r| r.0 == results[0].0));
    assert_eq!(results.iter().filter(|r| r.1).count(), 1);
}

#[test]
fn cancellation_and_failure_preserve_partial_segments() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("jobs.sqlite");
    let mut queue = Queue::open(&path).unwrap();
    let id = queue.enqueue(&spec()).unwrap();
    let lease = queue.claim("speech", None, TTL).unwrap().unwrap();
    queue.set_total(&lease, 2).unwrap();
    queue.append(&lease, &segment(0)).unwrap();
    assert!(matches!(queue.set_total(&lease, 3), Err(Error::Conflict)));
    let overlapping = Segment {
        start_ms: 500,
        ..segment(1)
    };
    assert!(matches!(
        queue.append(&lease, &overlapping),
        Err(Error::Conflict)
    ));
    queue.fail(&lease, "interrupted decoder").unwrap();
    assert_eq!(
        queue.get(id).unwrap().unwrap().error.as_deref(),
        Some("interrupted decoder")
    );
    queue.resume(id).unwrap();
    let lease = queue.claim("speech", None, TTL).unwrap().unwrap();
    queue.cancel(id).unwrap();
    assert!(matches!(
        queue.append(&lease, &segment(1)),
        Err(Error::Conflict)
    ));
    assert_eq!(queue.segments(id, 0, 10).unwrap(), [segment(0)]);
    assert_eq!(queue.get(id).unwrap().unwrap().state, State::Cancelled);
}

#[test]
fn refuses_unrelated_database_and_distinguishes_source_revisions() {
    let temp = tempfile::tempdir().unwrap();
    let unrelated = temp.path().join("notes.sqlite");
    Connection::open(&unrelated)
        .unwrap()
        .execute("CREATE TABLE notes (name TEXT)", [])
        .unwrap();
    assert!(matches!(Queue::open(&unrelated), Err(Error::Schema)));

    let queue = Queue::open(&temp.path().join("jobs.sqlite")).unwrap();
    let original = queue.enqueue(&spec()).unwrap();
    let changed = Spec {
        revision: "b".repeat(64),
        ..spec()
    };
    let newer = queue.enqueue(&changed).unwrap();
    assert_ne!(original, newer);
    assert_eq!(
        queue.list("speech", Some(newer), 10).unwrap()[0].id,
        original
    );
}
