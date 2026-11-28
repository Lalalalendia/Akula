use super::*;
use crate::background_jobs::{Segment, Spec, speech, speech::WINDOW_MS};
use std::time::Duration;

#[test]
fn version_one_checkpoints_survive_migration_and_edits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("speech.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../../schema.sql"))
        .unwrap();
    connection
        .pragma_update(None, "application_id", 0x414b4a31)
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    let mut old = Queue { connection };
    let job = old
        .enqueue(&Spec {
            kind: KIND.into(),
            source: "video".into(),
            revision: "a".repeat(64),
            recipe: "test".into(),
        })
        .unwrap();
    let lease = old
        .claim(KIND, Some(job), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    old.set_total(&lease, 2).unwrap();
    let cue = Cue {
        start_ms: 1_005,
        end_ms: 3_100,
        text: "original".into(),
    };
    let segment = Segment {
        ordinal: 0,
        start_ms: 0,
        end_ms: WINDOW_MS,
        payload: serde_json::to_vec(&[&cue]).unwrap(),
    };
    old.append(&lease, &segment).unwrap();
    old.cancel(job).unwrap();
    drop(old);

    let queue = Queue::open(&path).unwrap();
    let row = page(&queue, job, 0).unwrap().rows.remove(0);
    assert_eq!(row.cue, cue);
    assert_eq!(row.revision, 0);
    let saved = save(&queue, job, row.id, row.revision, "  First\n& <second>  ").unwrap();
    assert_eq!((saved.cue.start_ms, saved.cue.end_ms), (1_005, 3_100));
    assert!(save(&queue, job, row.id, row.revision, "old overwrite").is_err());
    assert!(
        save(
            &queue,
            job,
            CueId {
                window: 1,
                index: 0
            },
            0,
            "not committed"
        )
        .is_err()
    );
    assert_eq!(queue.segments(job, 0, 2).unwrap(), [segment]);
    drop(queue);

    let queue = Queue::open(&path).unwrap();
    assert_eq!(page(&queue, job, 0).unwrap().rows, [saved.clone()]);
    assert_eq!(
        speech::read_cues(&queue, job, 0, 1).unwrap(),
        [(0, vec![saved.cue])]
    );
    let mut vtt = Vec::new();
    speech::write_vtt(&queue, job, &mut vtt).unwrap();
    assert_eq!(
        String::from_utf8(vtt).unwrap(),
        "WEBVTT\n\n00:00:01.005 --> 00:00:03.100\nFirst &amp; &lt;second&gt;\n\n"
    );
    let suppressed = save(&queue, job, row.id, saved.revision, " ").unwrap();
    assert_eq!(page(&queue, job, 0).unwrap().rows, [suppressed]);
    assert_eq!(speech::read_cues(&queue, job, 0, 1).unwrap(), [(0, vec![])]);
}
