use super::*;
use crate::background_jobs::{Spec, State};

#[test]
fn automatic_claim_is_fifo_and_respects_pauses_and_live_owners() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("queue.sqlite");
    let queue = Queue::open(&path).unwrap();
    let other = Queue::open(&path).unwrap();
    let ids: Vec<_> = (0..4)
        .map(|index| {
            queue
                .enqueue(&Spec {
                    kind: "speech".into(),
                    source: format!("video-{index}.mp4"),
                    revision: "a".repeat(64),
                    recipe: "test".into(),
                })
                .unwrap()
        })
        .collect();
    let ttl = Duration::from_secs(30);
    queue
        .claim_at("speech", Some(ids[0]), ttl, 1)
        .unwrap()
        .unwrap();
    assert!(queue.claim_queued("speech", ids[0], ttl).unwrap().is_none());
    assert_eq!(queue.oldest_queued("speech").unwrap(), Some(ids[1]));

    let lease = queue.claim_queued("speech", ids[1], ttl).unwrap().unwrap();
    assert_eq!(lease.job(), ids[1]);
    assert!(other.claim_queued("speech", ids[2], ttl).unwrap().is_none());
    other.pause(ids[1]).unwrap();
    assert!(queue.claim_queued("speech", ids[1], ttl).unwrap().is_none());

    other.cancel(ids[2]).unwrap();
    assert_eq!(queue.oldest_queued("speech").unwrap(), Some(ids[3]));
    assert_eq!(
        other
            .claim_queued("speech", ids[3], ttl)
            .unwrap()
            .unwrap()
            .job(),
        ids[3]
    );
    assert_eq!(queue.get(ids[1]).unwrap().unwrap().state, State::Paused);
    assert_eq!(queue.oldest_queued("speech").unwrap(), None);
}
