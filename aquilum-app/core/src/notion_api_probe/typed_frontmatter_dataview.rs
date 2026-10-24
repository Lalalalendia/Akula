//! Exact integration: Notion API snapshot -> verified SQLite cache ->
//! Markdown candidate -> native Akula fields indexer -> Dataview execution.
//! No token, network, published Vault or user content is involved.
use super::key;
use crate::notion_api_probe::{
    projection::{prepare_import, ImportCandidate}, snapshot::SnapshotStore,
    Link, RawObject, SourcePreview, API_VERSION,
};
use crate::search::{
    dataview::{execute::{self, CellPart}, parser, rows::Row},
    fields::{note_fields, FieldKind},
};
use serde_json::json;

const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const C: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const D: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
const DS: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const OTHER_DS: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";

fn page(id: &str, ds: &str, score: f64, labels: &[&str]) -> RawObject {
    RawObject {
        kind: "page".into(), id: id.into(),
        json: json!({
            "object": "page", "id": id,
            "parent": {"type":"data_source_id", "data_source_id": ds},
            "properties": {
                "Name": {"id":"name", "type":"title",
                    "title":[{"plain_text":"Imported row"}]},
                "Score": {"id":"score-prop", "type":"number", "number":score},
                "Done": {"id":"done-prop", "type":"checkbox", "checkbox":true},
                "Labels": {"id":"labels-prop", "type":"multi_select",
                    "multi_select":labels.iter().map(|name| json!({"name":name})).collect::<Vec<_>>()},
                "Computed": {"id":"formula-prop", "type":"formula",
                    "formula":{"number":1000}}
            }
        }),
    }
}
fn data_source(id: &str) -> RawObject {
    RawObject {
        kind:"data_source".into(), id:id.into(),
        json:json!({"object":"data_source","id":id,"properties":{
            "Name":{"id":"name","type":"title"},
            "Score":{"id":"score-prop","type":"number"},
            "Labels":{"id":"labels-prop","type":"multi_select"},
            "Computed":{"id":"formula-prop","type":"formula"}
        }}),
    }
}
fn snapshot() -> SourcePreview {
    let pages = [
        page(A, DS, 8.0, &["Blocked"]),
        page(B, DS, 42.5, &["Research", "Needs review"]),
        page(C, OTHER_DS, 99.0, &["Research"]),
        page(D, OTHER_DS, 6.0, &["Blocked"]),
    ];
    let mut objects = vec![data_source(DS), data_source(OTHER_DS)];
    for page in pages {
        objects.push(RawObject {
            kind:"markdown".into(), id:page.id.clone(),
            json:json!({"markdown":"A body from the Notion API"}),
        });
        objects.push(page);
    }
    SourcePreview {
        format:"notion_api_dataview_test", api_version:API_VERSION, root:A.into(),
        workspace_complete:false, root_traversal_complete:true, requests:9,
        objects,
        links:vec![
            Link{from:B.into(),to:A.into(),kind:"relation".into(),
                property_id:Some("linked-prop".into())},
            Link{from:C.into(),to:D.into(),kind:"relation".into(),
                property_id:Some("linked-prop".into())},
        ],
        unresolved_target_ids:vec![], warnings:vec![],
    }
}
fn as_row(candidate: ImportCandidate) -> Row {
    let relative = candidate.relative_path.clone();
    Row {
        name:candidate.source_id.clone(),
        source:format!("/test-vault/{relative}"),
        folder:"Notion-import".into(), relative,
        created:0, modified:0, size:candidate.markdown.len() as i64,
        fields:note_fields(&candidate.markdown), tasks:vec![],
    }
}
fn run(query: &str, rows: Vec<Row>) -> execute::QueryOutput {
    execute::run(&parser::parse(query).expect("parse Dataview"), rows)
        .expect("execute Dataview")
}
fn cell_text(cell: &execute::Cell) -> String {
    cell.parts.iter().map(|part| match part {
        CellPart::Text{text} | CellPart::Link{text,..} => text.clone(),
        CellPart::Progress{percent} => format!("{percent}%"),
        CellPart::Check{done,..} => done.to_string(),
    }).collect()
}
#[test]
fn api_cache_to_dataview_filters_by_data_source_and_sorts_numeric_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("ui-state.sqlite3");
    let expected = {
        let mut writer = SnapshotStore::open(&cache).unwrap();
        writer.publish(&snapshot(), 123).unwrap().digest
    };
    let store = SnapshotStore::open_read_only(&cache).unwrap();
    let saved = store.load(A).unwrap().expect("verified source cache");
    assert_eq!(saved.info.digest, expected);
    assert_eq!(saved.info.object_count, 10);
    assert_eq!(saved.info.link_count, 2);
    let imported = prepare_import(&saved.into_preview()).unwrap();
    assert!(imported.publish_allowed);
    assert_eq!(imported.candidates.len(), 4);
    assert!(imported.unresolved.is_empty());
    let related = imported.candidates.iter().filter(|page| !page.relations.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(related.len(), 2);
    assert!(imported.candidates.iter().find(|p| p.source_id == B)
        .unwrap().markdown.contains(&format!("<{A}.md>")));
    assert!(imported.candidates.iter().find(|p| p.source_id == C)
        .unwrap().markdown.contains(&format!("<{D}.md>")));
    let make_rows = || imported.candidates.iter().cloned().map(as_row).collect::<Vec<_>>();
    let score = key("Score", &json!({"id":"score-prop"})).unwrap();
    let labels = key("Labels", &json!({"id":"labels-prop"})).unwrap();
    let formula = key("Computed", &json!({"id":"formula-prop"})).unwrap();
    for row in make_rows() {
        assert_eq!(row.field(&score).unwrap().kind, FieldKind::Number);
        assert!(row.field(&formula).is_none(), "formulas must not be approximated");
        assert_eq!(row.field("notion_query_projection_partial").unwrap().text, "true");
    }
    let sorted = format!(
        "TABLE WITHOUT ID {score} WHERE notion_data_source_id = \"{DS}\" SORT {score} DESC"
    );
    let table = run(&sorted, make_rows());
    assert_eq!(table.shape, "table");
    assert_eq!(table.width, 1);
    assert_eq!(table.total, 2);
    assert_eq!(cell_text(&table.rows[0]), "42.5");
    assert_eq!(cell_text(&table.rows[1]), "8");
    let filter = format!(
        "TABLE WITHOUT ID file.name WHERE notion_data_source_id = \"{DS}\" \
         AND contains({labels}, \"Research\")"
    );
    let found = run(&filter, make_rows());
    assert_eq!(found.total, 1);
    assert_eq!(cell_text(&found.rows[0]), B);
}

#[test]
fn file_gate_import_is_searchable_through_actual_sqlite_dataview_index() {
    use crate::app_core::{Core, CoreEvent};
    use crate::notion_api_probe::{
        cached_import::approved_cached_preview, publish, structured_bundle,
    };
    use std::{fs, sync::Arc, time::{Duration, Instant}};

    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    let vault = temp.path().join("vault");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&vault).unwrap();
    let cache = app.join("ui-state.sqlite3");
    let digest = {
        let mut store = SnapshotStore::open(&cache).unwrap();
        store.publish(&snapshot(), 400).unwrap().digest
    };
    let preview = approved_cached_preview(&cache, A, &digest).unwrap();
    let bundle = structured_bundle::prepare_from_cached(&cache, A, &digest).unwrap();
    assert_eq!(preview.candidates.len(), 4);
    assert_eq!(bundle.review().edge_count, 2);
    assert_eq!(bundle.review().object_counts.get("data_source"), Some(&2));

    let core = Core::open(&app, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
    assert!(publish::publish_approved(&core, &vault, &preview, false).is_err());
    assert!(!vault.join("Notion-import").exists());
    let first = publish::publish_approved(&core, &vault, &preview, true).unwrap();
    assert!(first.completed);
    assert_eq!(first.created.len(), 4);
    let structured = structured_bundle::publish_approved(&core, &vault, &bundle, true).unwrap();
    let verified_bundle = structured_bundle::inspect_existing(&vault.join(&structured.relative_path))
        .expect("independent typed JSON survives File Gate and readback");
    assert_eq!(verified_bundle.source_digest, digest);
    assert_eq!(verified_bundle.edge_count, 2);
    let repeated = publish::publish_approved(&core, &vault, &preview, true).unwrap();
    assert!(repeated.completed && repeated.created.is_empty());
    assert_eq!(repeated.existing_untouched.len(), 4);

    // Unlike the pure parser test above, this query uses the on-disk SQLite
    // metadata populated by SearchService's real asynchronous indexing worker.
    let root = vault.to_str().unwrap();
    core.search.prepare(root).unwrap();
    let score = key("Score", &json!({"id":"score-prop"})).unwrap();
    let labels = key("Labels", &json!({"id":"labels-prop"})).unwrap();
    let query = format!(
        "TABLE WITHOUT ID {score} WHERE notion_data_source_id = \"{DS}\" SORT {score} DESC"
    );
    let origin = vault.join(format!("Notion-import/{A}.md"));
    let origin = origin.to_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let table = loop {
        let attempt = core.search.run_dataview_query(root, origin, &query, 0);
        if let Ok(found) = &attempt {
            if found.total == 2 { break found.clone(); }
        }
        assert!(Instant::now() < deadline, "File Gate import was not indexed: {attempt:?}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(table.total, 2);
    assert_eq!(cell_text(&table.rows[0]), "42.5");
    assert_eq!(cell_text(&table.rows[1]), "8");

    let filtered = format!(
        "TABLE WITHOUT ID file.name WHERE notion_data_source_id = \"{DS}\" \
         AND contains({labels}, \"Research\")"
    );
    let selected = core.search.run_dataview_query(root, origin, &filtered, 0).unwrap();
    assert_eq!(selected.total, 1);
    assert_eq!(cell_text(&selected.rows[0]), B);
    core.search.deactivate().unwrap();
    assert_eq!(fs::read_to_string(vault.join(format!("Notion-import/{A}.md"))).unwrap(),
        preview.candidates.iter().find(|p| p.source_id == A).unwrap().markdown);
}
