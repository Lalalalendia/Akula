//! Exact integration: Notion API snapshot -> verified SQLite cache ->
//! Markdown candidate -> native Akula fields indexer -> Dataview execution.
//! No token, network, published Vault or user content is involved.
use super::key;
use crate::notion_api_probe::{
    projection::{prepare_import, ImportCandidate}, snapshot::SnapshotStore,
    RawObject, SourcePreview, API_VERSION,
};
use crate::search::{
    dataview::{execute::{self, CellPart}, parser, rows::Row},
    fields::{note_fields, FieldKind},
};
use serde_json::json;

const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const C: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
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
fn snapshot() -> SourcePreview {
    let pages = [
        page(A, DS, 8.0, &["Blocked"]),
        page(B, DS, 42.5, &["Research", "Needs review"]),
        page(C, OTHER_DS, 99.0, &["Research"]),
    ];
    let mut objects = Vec::new();
    for page in pages {
        objects.push(RawObject {
            kind:"markdown".into(), id:page.id.clone(),
            json:json!({"markdown":"A body from the Notion API"}),
        });
        objects.push(page);
    }
    SourcePreview {
        format:"notion_api_dataview_test", api_version:API_VERSION, root:A.into(),
        workspace_complete:false, root_traversal_complete:true, requests:7,
        objects, links:vec![], unresolved_target_ids:vec![], warnings:vec![],
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
    let imported = prepare_import(&saved.into_preview()).unwrap();
    assert!(imported.publish_allowed);
    assert_eq!(imported.candidates.len(), 3);
    assert!(imported.unresolved.is_empty());
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
