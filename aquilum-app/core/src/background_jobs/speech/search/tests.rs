use super::*;
use crate::background_jobs::speech::{Cue, WINDOW_MS, fingerprint};
use crate::background_jobs::{Segment, Spec};
use std::{fs, time::Duration};

fn seed(queue: &mut Queue, root: &Path, name: &str, text: &str, finish: bool) -> (JobId, PathBuf) {
    fs::create_dir_all(root).unwrap();
    let path = root.join(name);
    fs::write(&path, b"original media").unwrap();
    let path = path.canonicalize().unwrap();
    let digest = fingerprint(&path, &mut || Ok(())).unwrap().digest;
    let id = queue
        .enqueue(&Spec {
            kind: KIND.into(),
            source: path.to_str().unwrap().into(),
            revision: digest,
            recipe: "test".into(),
        })
        .unwrap();
    let lease = queue
        .claim(KIND, Some(id), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    queue.set_total(&lease, 1).unwrap();
    queue
        .append(
            &lease,
            &Segment {
                ordinal: 0,
                start_ms: 0,
                end_ms: WINDOW_MS,
                payload: serde_json::to_vec(&[Cue {
                    start_ms: 2560,
                    end_ms: 3930,
                    text: text.into(),
                }])
                .unwrap(),
            },
        )
        .unwrap();
    if finish {
        queue.finish(&lease).unwrap();
    }
    (id, path)
}

#[test]
fn completed_speech_is_scoped_and_edits_replace_indexed_text() {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    let library = temp.path().join("private-media");
    let mut queue = Queue::open(&temp.path().join("jobs.sqlite")).unwrap();
    let (id, source) = seed(&mut queue, &vault, "target.mp4", "Полярная станция", true);
    let (video, _) = seed(&mut queue, &library, "local.mkv", "Полярная станция", true);
    seed(
        &mut queue,
        &vault,
        "unfinished.mp4",
        "Полярная станция",
        false,
    );
    seed(
        &mut queue,
        &temp.path().join("vault-other"),
        "foreign.mp4",
        "Полярная станция",
        true,
    );
    let vault = vault.canonicalize().unwrap();
    let library = library.canonicalize().unwrap();
    let mut session = Session::default();
    let cancel = Cancel::never();
    let found = session
        .search(
            &queue,
            "полярная стан",
            Some(&vault),
            Some(&library),
            &cancel,
        )
        .unwrap();
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|hit| hit.target.job == video));
    let hit = found.iter().find(|hit| hit.target.job == id).unwrap();
    assert!(hit.snippet.contains("Полярная"));
    let (page, _, time) = locate(&queue, hit.target).unwrap();
    assert_eq!(time, 2560);
    assert_eq!(page.rows[0].id, hit.target.cue);

    let corrected = edits::save(
        &queue,
        id,
        hit.target.cue,
        hit.target.revision,
        "Северная база",
    )
    .unwrap();
    assert!(locate(&queue, hit.target).is_err());
    let only_vault = |session: &mut Session, query: &str| {
        session
            .search(&queue, query, Some(&vault), None, &cancel)
            .unwrap()
    };
    assert!(only_vault(&mut session, "полярная").is_empty());
    let current = only_vault(&mut session, "северная").pop().unwrap();
    assert_eq!(current.target.revision, corrected.revision);
    edits::save(&queue, id, current.target.cue, current.target.revision, "").unwrap();
    assert!(only_vault(&mut session, "северная").is_empty());
    let restored = edits::save(
        &queue,
        id,
        current.target.cue,
        current.target.revision + 1,
        "новая фраза",
    )
    .unwrap();
    let found = only_vault(&mut session, "новая");
    assert_eq!(found[0].target.revision, restored.revision);
    fs::write(source, b"different media").unwrap();
    assert!(only_vault(&mut session, "новая").is_empty());
    assert!(locate(&queue, found[0].target).is_err());
}

#[test]
fn version_two_backfill_uses_saved_corrections() {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    let db = temp.path().join("legacy.sqlite");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!("../../schema.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../schema_v2.sql"))
        .unwrap();
    conn.pragma_update(None, "application_id", 0x414b4a31)
        .unwrap();
    conn.pragma_update(None, "user_version", 2).unwrap();
    let mut queue = Queue { connection: conn };
    let (id, _) = seed(&mut queue, &vault, "old.mp4", "Original phrase", true);
    let cue = edits::CueId {
        window: 0,
        index: 0,
    };
    edits::save(&queue, id, cue, 0, "Corrected phrase").unwrap();
    drop(queue);
    let queue = Queue::open(&db).unwrap();
    let mut session = Session::default();
    let vault = vault.canonicalize().unwrap();
    let find = |session: &mut Session, text| {
        session
            .search(&queue, text, Some(&vault), None, &Cancel::never())
            .unwrap()
    };
    assert!(find(&mut session, "original").is_empty());
    let repaired = find(&mut session, "corrected");
    assert_eq!(
        repaired[0].target,
        Target {
            job: id,
            cue,
            revision: 1
        }
    );
}
