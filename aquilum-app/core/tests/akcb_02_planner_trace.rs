//! AKCB-02: test-only trace from the real Sync Planner, no File Gate / network / grant enablement.
//! The external Python checker must independently validate all reported decisions.

use aquilum_core::sync::{
    plan_snapshot, DocumentId, IngestOutcome, ObservedObject, PlannedAction, RevisionDag,
    RevisionId, RevisionOperation, RevisionRecord, SafetyPolicy, SnapshotDiff, SyncRelativePath,
};
use serde_json::{json, Value};

const LABELS: [&str; 12] = [
    "root", "left", "right", "tombstone", "unrelated", "foreign", "bad_hash", "missing",
    "unknown", "inaccessible", "excluded", "placeholder",
];

struct Fixture {
    dag: RevisionDag,
    doc: DocumentId,
    root: RevisionId,
    left: RevisionId,
    right: RevisionId,
    tombstone: RevisionId,
    unrelated: RevisionId,
    foreign: RevisionId,
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn add(
    dag: &mut RevisionDag,
    doc: DocumentId,
    parents: Vec<RevisionId>,
    operation: RevisionOperation,
) -> RevisionId {
    let id = RevisionId::new();
    let result = dag.ingest(RevisionRecord { id, document_id: doc, parents, operation });
    assert_eq!(result, Ok(IngestOutcome::Activated(vec![id])));
    id
}

fn content(dag: &mut RevisionDag, doc: DocumentId, parents: Vec<RevisionId>, bytes: &[u8]) -> RevisionId {
    add(
        dag,
        doc,
        parents,
        RevisionOperation::Content {
            path: SyncRelativePath::parse("notes/akcb-02.md").unwrap(),
            blob_hash: digest(bytes),
        },
    )
}

impl Fixture {
    fn new() -> Self {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let other_doc = DocumentId::new();
        let root = content(&mut dag, doc, vec![], b"root");
        let left = content(&mut dag, doc, vec![root], b"left");
        let right = content(&mut dag, doc, vec![root], b"right");
        let tombstone = add(
            &mut dag,
            doc,
            vec![root],
            RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/akcb-02.md").unwrap(),
            },
        );
        let unrelated = content(&mut dag, doc, vec![], b"unrelated");
        let foreign = content(&mut dag, other_doc, vec![], b"foreign");
        Self { dag, doc, root, left, right, tombstone, unrelated, foreign }
    }

    fn observation(&self, label: &str) -> ObservedObject {
        let present = |revision, bytes: &[u8]| ObservedObject::Present {
            revision,
            content_hash: digest(bytes),
        };
        match label {
            "root" => present(self.root, b"root"),
            "left" => present(self.left, b"left"),
            "right" => present(self.right, b"right"),
            "unrelated" => present(self.unrelated, b"unrelated"),
            "foreign" => present(self.foreign, b"foreign"),
            "bad_hash" => present(self.root, b"tampered"),
            "tombstone" => ObservedObject::Tombstone { revision: self.tombstone },
            "missing" => ObservedObject::VerifiedAbsent,
            "unknown" => ObservedObject::Unknown,
            "inaccessible" => ObservedObject::Inaccessible,
            "excluded" => ObservedObject::Excluded,
            "placeholder" => ObservedObject::Placeholder,
            _ => panic!("unknown observation label"),
        }
    }
}

fn observed_revision(value: &ObservedObject) -> Option<RevisionId> {
    match value {
        ObservedObject::Present { revision, .. } | ObservedObject::Tombstone { revision } => {
            Some(*revision)
        }
        _ => None,
    }
}

fn observed_action(action: PlannedAction, local: &ObservedObject, remote: &ObservedObject) -> String {
    match action {
        PlannedAction::NoChange => "no_change".to_string(),
        PlannedAction::Hold(reason) => format!("hold:{reason:?}"),
        PlannedAction::UploadImmutable { revision } => {
            assert_eq!(Some(revision), observed_revision(local), "wrong upload target");
            "upload_immutable".to_string()
        }
        PlannedAction::StageDownload { revision } => {
            assert_eq!(Some(revision), observed_revision(remote), "wrong download target");
            "stage_download".to_string()
        }
        PlannedAction::PreserveConflict { local: left, remote: right } => {
            assert_eq!(Some(left), observed_revision(local), "wrong local conflict head");
            assert_eq!(Some(right), observed_revision(remote), "wrong remote conflict head");
            "preserve_conflict".to_string()
        }
    }
}

#[test]
fn emit_real_sync_planner_trace_without_mutating_user_data() {
    let fixture = Fixture::new();
    let mut rows: Vec<Value> = Vec::with_capacity(LABELS.len() * LABELS.len() * 8);
    for &local_label in &LABELS {
        for &remote_label in &LABELS {
            let local = fixture.observation(local_label);
            let remote = fixture.observation(remote_label);
            for baseline in [false, true] {
                for upload in [false, true] {
                    for download in [false, true] {
                        let action = plan_snapshot(
                            &SnapshotDiff {
                                document_id: fixture.doc,
                                confirmed_baseline: if baseline { Some(fixture.root) } else { None },
                                local: local.clone(),
                                remote: remote.clone(),
                            },
                            &fixture.dag,
                            SafetyPolicy {
                                allow_initial_upload: upload,
                                allow_initial_download: download,
                            },
                        );
                        let outcome = observed_action(action, &local, &remote);
                        rows.push(json!({
                            "local": local_label,
                            "remote": remote_label,
                            "baseline": baseline,
                            "allow_initial_upload": upload,
                            "allow_initial_download": download,
                            "outcome": outcome,
                        }));
                    }
                }
            }
        }
    }
    assert_eq!(rows.len(), 1152);
    let trace = json!({
        "schema": "akcb-02/sync-plan-trace/v1",
        "target": "aquilum_core::sync::plan_snapshot",
        "cases": rows,
    });
    if let Some(path) = std::env::var_os("AKCB_02_TRACE_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&trace).unwrap())
            .expect("write test-only AKCB trace");
    }
    println!("AKCB_02_RUST_PLANNER_TRACE_EMITTED cases=1152");
}
