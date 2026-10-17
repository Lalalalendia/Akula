use super::database::UiStateDatabase;
use super::models::{
    GraphCameraState, OpenSessionInput, SaveStateBatchInput, SessionStateInput, TabKind, TabState,
    ViewStateInput,
};
use uuid::Uuid;

pub fn setup() -> (UiStateDatabase, Uuid, Uuid, Uuid) {
    let mut database = UiStateDatabase::memory().expect("database");
    let workspace_id = database
        .resolve_workspace("C:/notes", 1)
        .expect("workspace");
    let document_id = database
        .resolve_document(workspace_id, "note.md")
        .expect("document");
    let epoch = Uuid::new_v4();
    database
        .open_session(&OpenSessionInput {
            workspace_id,
            window_id: "main".to_owned(),
            epoch,
            now_ms: 2,
        })
        .expect("session");
    (database, workspace_id, document_id, epoch)
}

#[test]
fn lists_known_workspaces_most_recently_seen_first() {
    let (mut database, _workspace_id, _document_id, _epoch) = setup();
    database.resolve_workspace("C:/archive", 9).expect("archive");
    database.resolve_workspace("C:/drafts", 5).expect("drafts");

    let paths = database
        .list_workspaces()
        .expect("workspaces")
        .into_iter()
        .map(|workspace| workspace.path)
        .collect::<Vec<_>>();

    assert_eq!(paths, ["C:/archive", "C:/drafts", "C:/notes"]);
}

#[test]
fn forgetting_a_workspace_removes_it_from_the_list() {
    let (mut database, workspace_id, _document_id, _epoch) = setup();
    database.resolve_workspace("C:/drafts", 5).expect("drafts");

    database.forget_workspace(workspace_id).expect("forget");

    let paths = database
        .list_workspaces()
        .expect("workspaces")
        .into_iter()
        .map(|workspace| workspace.path)
        .collect::<Vec<_>>();

    assert_eq!(paths, ["C:/drafts"]);
}

#[test]
fn listed_paths_drop_the_windows_verbatim_prefix() {
    use super::paths::display_workspace;

    assert_eq!(
        display_workspace(r"\\?\D:\База знаний копия\NeuroNet"),
        r"D:\База знаний копия\NeuroNet"
    );
    assert_eq!(display_workspace(r"\\?\UNC\server\share"), r"\\server\share");
    assert_eq!(display_workspace("/home/user/notes"), "/home/user/notes");
}

fn view(document_id: Uuid, position: i64) -> ViewStateInput {
    ViewStateInput {
        path: None,
        document_id,
        pane_id: "main".to_owned(),
        cursor_anchor: vec![1, 2],
        cursor_head: vec![1, 3],
        fallback_anchor: position,
        fallback_head: position,
        scroll_anchor: vec![4, 5],
        fallback_scroll_anchor: position,
        scroll_offset_px: 12.5,
        focused_surface: "body".to_owned(),
    }
}

pub fn batch(
    workspace_id: Uuid,
    document_id: Uuid,
    epoch: Uuid,
    sequence: i64,
    position: i64,
) -> SaveStateBatchInput {
    let tab_id = Uuid::new_v4();
    SaveStateBatchInput {
        workspace_id,
        window_id: "main".to_owned(),
        epoch,
        sequence,
        now_ms: sequence + 10,
        session: Some(SessionStateInput {
            active_tab_id: Some(tab_id),
            tabs: Some(vec![TabState {
                tab_id,
                document_id: Some(document_id),
                kind: TabKind::Document,
                position: 0,
            }]),
        }),
        views: vec![view(document_id, position)],
        graph_camera: None,
    }
}

#[test]
fn stale_sequence_cannot_replace_newer_state() {
    let (mut database, workspace_id, document_id, epoch) = setup();
    assert!(database
        .save_batch(&batch(workspace_id, document_id, epoch, 2, 20))
        .expect("new batch"));
    assert!(!database
        .save_batch(&batch(workspace_id, document_id, epoch, 1, 10))
        .expect("stale batch"));
    let loaded = database.load_session(workspace_id, "main").expect("load");
    assert_eq!(loaded.views[0].fallback_anchor, 20);
}

#[test]
fn previous_epoch_cannot_write_after_session_reopens() {
    let (mut database, workspace_id, document_id, old_epoch) = setup();
    let new_epoch = Uuid::new_v4();
    database
        .open_session(&OpenSessionInput {
            workspace_id,
            window_id: "main".to_owned(),
            epoch: new_epoch,
            now_ms: 20,
        })
        .expect("reopen");
    assert!(!database
        .save_batch(&batch(workspace_id, document_id, old_epoch, 99, 99))
        .expect("old epoch"));
    assert!(database
        .save_batch(&batch(workspace_id, document_id, new_epoch, 1, 30))
        .expect("new epoch"));
}

#[test]
fn missing_document_is_retained_until_cutoff_then_cascades() {
    let (mut database, workspace_id, document_id, epoch) = setup();
    database
        .save_batch(&batch(workspace_id, document_id, epoch, 1, 20))
        .expect("batch");
    database
        .mark_document_missing(document_id, 100)
        .expect("missing");
    assert_eq!(database.purge_missing(99, 500).expect("early purge"), 0);
    let replacement = database
        .resolve_document(workspace_id, "note.md")
        .expect("replacement");
    assert_ne!(replacement, document_id);
    assert_eq!(database.purge_missing(100, 500).expect("purge"), 1);
    let loaded = database.load_session(workspace_id, "main").expect("load");
    assert!(loaded.tabs.is_empty());
    assert!(loaded.views.is_empty());
    assert!(loaded.active_tab_id.is_none());
    let resolved = database
        .resolve_document(workspace_id, "note.md")
        .expect("resolve replacement");
    assert_eq!(resolved, replacement);
}

#[test]
fn rename_preserves_document_identity() {
    let (mut database, workspace_id, document_id, _) = setup();
    database
        .rename_document(document_id, "renamed.md")
        .expect("rename");
    let resolved = database
        .resolve_document(workspace_id, "renamed.md")
        .expect("resolve");
    assert_eq!(resolved, document_id);
}

#[test]
fn zero_cleanup_limit_does_not_delete_tombstone() {
    let (mut database, _, document_id, _) = setup();
    database
        .mark_document_missing(document_id, 100)
        .expect("missing");
    assert_eq!(database.purge_missing(100, 0).expect("purge"), 0);
}

#[test]
fn session_survives_database_reopen() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("ui-state.sqlite3");
    let workspace_id;
    let document_id;
    {
        let mut database = UiStateDatabase::open(&path).expect("open");
        workspace_id = database
            .resolve_workspace("C:/notes", 1)
            .expect("workspace");
        document_id = database
            .resolve_document(workspace_id, "note.md")
            .expect("document");
        let epoch = Uuid::new_v4();
        database
            .open_session(&OpenSessionInput {
                workspace_id,
                window_id: "main".to_owned(),
                epoch,
                now_ms: 2,
            })
            .expect("session");
        database
            .save_batch(&batch(workspace_id, document_id, epoch, 1, 42))
            .expect("save");
    }
    let database = UiStateDatabase::open(&path).expect("reopen");
    let loaded = database.load_session(workspace_id, "main").expect("load");
    assert_eq!(loaded.tabs[0].document_id, Some(document_id));
    assert_eq!(loaded.tabs[0].relative_path.as_deref(), Some("note.md"));
    assert_eq!(loaded.views[0].fallback_anchor, 42);
    assert_eq!(loaded.views[0].fallback_scroll_anchor, 42);
}

#[test]
fn the_graph_camera_survives_a_reopened_session() {
    let (mut database, workspace_id, document_id, epoch) = setup();
    let mut input = batch(workspace_id, document_id, epoch, 1, 0);
    input.graph_camera = Some(GraphCameraState {
        center_x: -3.5,
        center_y: 7.25,
        scale: 42.0,
    });
    assert!(database.save_batch(&input).expect("saved"));

    let reopened = database
        .open_session(&OpenSessionInput {
            workspace_id,
            window_id: "main".to_owned(),
            epoch: Uuid::new_v4(),
            now_ms: 20,
        })
        .expect("session");

    assert_eq!(
        reopened.graph_camera,
        Some(GraphCameraState {
            center_x: -3.5,
            center_y: 7.25,
            scale: 42.0,
        })
    );
}

#[test]
fn a_session_without_a_graph_camera_reports_none() {
    let (mut database, workspace_id, document_id, epoch) = setup();
    assert!(database
        .save_batch(&batch(workspace_id, document_id, epoch, 1, 0))
        .expect("saved"));

    let reopened = database
        .open_session(&OpenSessionInput {
            workspace_id,
            window_id: "main".to_owned(),
            epoch: Uuid::new_v4(),
            now_ms: 20,
        })
        .expect("session");

    assert_eq!(reopened.graph_camera, None);
}

#[test]
fn repeated_multi_row_batches_rebind_every_tab_and_view() {
    let (mut database, workspace_id, document_id, epoch) = setup();
    let other = database.resolve_document(workspace_id, "other.md").expect("other");
    for sequence in 1..=3 {
        let mut input = batch(workspace_id, document_id, epoch, sequence, sequence * 10);
        input.session.as_mut().unwrap().tabs.as_mut().unwrap().push(TabState {
            tab_id: Uuid::new_v4(),
            document_id: Some(other),
            kind: TabKind::Document,
            position: 1,
        });
        input.views.push(view(other, sequence * 100));
        assert!(database.save_batch(&input).expect("save"));
        let loaded = database.load_session(workspace_id, "main").expect("load");
        assert_eq!(loaded.tabs.len(), 2);
        assert_eq!(loaded.tabs[0].document_id, Some(document_id));
        assert_eq!(loaded.tabs[1].document_id, Some(other));
        assert_eq!(loaded.views.len(), 2);
        let mut positions: Vec<_> = loaded.views.iter().map(|v| v.fallback_anchor).collect();
        positions.sort();
        assert_eq!(positions, [sequence * 10, sequence * 100]);
    }
}

#[test]
fn failed_second_view_rolls_back_tabs_first_view_and_sequence() {
    let (mut database, workspace_id, document_id, epoch) = setup();
    let initial = batch(workspace_id, document_id, epoch, 1, 10);
    let initial_active = initial.session.as_ref().unwrap().active_tab_id;
    assert!(database.save_batch(&initial).expect("initial"));
    let mut failed = batch(workspace_id, document_id, epoch, 2, 20);
    failed.views.push(view(Uuid::new_v4(), 99));
    assert!(database.save_batch(&failed).is_err());
    let loaded = database.load_session(workspace_id, "main").expect("load");
    assert_eq!(loaded.active_tab_id, initial_active);
    assert_eq!(loaded.tabs.len(), 1);
    assert_eq!(loaded.views.len(), 1);
    assert_eq!(loaded.views[0].fallback_anchor, 10);
    failed.views.pop();
    assert!(database.save_batch(&failed).expect("retry same sequence"));
    assert_eq!(database.load_session(workspace_id, "main").expect("load").views[0].fallback_anchor, 20);
}
