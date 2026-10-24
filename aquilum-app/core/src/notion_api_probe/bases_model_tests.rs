use super::*;
use crate::notion_api_probe::{RawObject, SourcePreview};
use serde_json::json;

const ROOT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const PAGE_B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const PAGE_C: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const PAGE_D: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const DS1: &str = "11111111-1111-4111-8111-111111111111";
const DS2: &str = "22222222-2222-4222-8222-222222222222";
const VIEW_A: &str = "33333333-3333-4333-8333-333333333333";
const VIEW_B: &str = "44444444-4444-4444-8444-444444444444";
const DIGEST: &str = "abcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd";

fn data_source(id:&str)->RawObject {
    RawObject {kind:"data_source".into(),id:id.into(),
        json:json!({"object":"data_source","id":id,"name":"Research database",
            "properties":{
                "Name":{"id":"title","type":"title"},
                "Score":{"id":"number%3Ascore","type":"number",
                    "number":{"format":"number"}},
                "Linked":{"id":"relation%3Aptr","type":"relation",
                    "relation":{"data_source_id":DS2}},
                "Calculated":{"id":"formula","type":"formula",
                    "formula":{"expression":"prop(\"Score\") + 1"}},
                "Aggregate":{"id":"rollup","type":"rollup",
                    "rollup":{"relation_property_id":"relation%3Aptr",
                        "rollup_property_id":"number%3Ascore",
                        "function":"sum"}}
            }})}
}
fn row(id:&str, owner:&str, score:i64)->RawObject {
    RawObject {kind:"page".into(),id:id.into(),
        json:json!({"object":"page","id":id,
            "last_edited_time":"2026-10-24T21:30:00Z",
            "parent":{"type":"data_source_id","data_source_id":owner},
            "properties":{
                "Name":{"id":"title","type":"title",
                    "title":[{"plain_text":"Same title"}]},
                "Score":{"id":"number%3Ascore","type":"number","number":score},
                "Linked":{"id":"relation%3Aptr","type":"relation",
                    "relation":[{"id":PAGE_B}]},
                "Calculated":{"id":"formula","type":"formula",
                    "formula":{"number":score+1}},
                "Aggregate":{"id":"rollup","type":"rollup",
                    "rollup":{"type":"number","number":55}}
            }})}
}
fn view(id:&str,owner:&str,kind:&str)->RawObject {
    RawObject {kind:"view".into(),id:id.into(),
        json:json!({"object":"view","id":id,"type":kind,
            "parent":{"type":"data_source_id","data_source_id":owner},
            "sorts":[{"property":"Score","direction":"descending"}],
            "filter":{"property":"Score","number":{"greater_than":10}},
            "group_by":{"property":"Name"}})}
}
fn fixture()->SourcePreview {
    SourcePreview{
        format:"test_api_snapshot",api_version:API_VERSION,root:ROOT.into(),
        workspace_complete:false,root_traversal_complete:true,requests:15,
        objects:vec![
            data_source(DS1),data_source(DS2),
            row(ROOT,DS1,8),row(PAGE_B,DS1,42),
            row(PAGE_C,DS2,21),row(PAGE_D,DS2,99),
            view(VIEW_A,DS1,"table"),view(VIEW_B,DS1,"board"),
            RawObject {kind:"page".into(),id:"55555555-5555-4555-8555-555555555555".into(),
                json:json!({"object":"page","id":"55555555-5555-4555-8555-555555555555",
                    "parent":{"type":"page_id","page_id":ROOT}})},
            // SnapshotStore requires one, and only one, Markdown payload for
            // every page (including a standalone page in the same root).
            RawObject {kind:"markdown".into(),id:ROOT.into(),
                json:json!({"markdown":"Body and task list"})},
            RawObject {kind:"markdown".into(),id:PAGE_B.into(),
                json:json!({"markdown":"Second row body"})},
            RawObject {kind:"markdown".into(),id:PAGE_C.into(),
                json:json!({"markdown":"Third row body"})},
            RawObject {kind:"markdown".into(),id:PAGE_D.into(),
                json:json!({"markdown":"Fourth row body"})},
            RawObject {kind:"markdown".into(),
                id:"55555555-5555-4555-8555-555555555555".into(),
                json:json!({"markdown":"Standalone note body"})},
        ],
        links:vec![
            super::super::Link{from:ROOT.into(),to:PAGE_B.into(),
                kind:"relation".into(),property_id:Some("relation%3Aptr".into())},
            super::super::Link{from:PAGE_C.into(),to:PAGE_D.into(),
                kind:"relation".into(),property_id:Some("relation%3Aptr".into())},
        ],
        unresolved_target_ids:vec![],warnings:vec![],
    }
}
fn find<'a>(model:&'a BasesModel, id:&str)->&'a BaseRow {
    model.rows.iter().find(|r|r.notion_page_id==id).unwrap()
}
fn cell<'a>(row:&'a BaseRow,property:&str)->&'a BaseCell {
    row.cells.iter().find(|c|c.notion_property_id.as_deref()==Some(property)).unwrap()
}

#[test]
fn normalized_data_sources_rows_views_and_inert_formulas_remain_independent() {
    let model=compose(&fixture(),DIGEST).unwrap();
    assert_eq!(model.bases.len(),2);
    assert_eq!(model.rows.len(),4);
    assert_eq!(model.views.len(),2);
    assert_eq!(model.unbound_rows,0);
    assert_eq!(model.unbound_views,0);
    assert_eq!(model.fields_needing_review,0);
    assert_eq!(model.source_relation_edges,2);
    assert!(!model.native_edit_supported);
    assert!(!model.views_rendered_natively);
    assert!(!model.workspace_identity_verified);
    assert!(!model.cross_root_merge_supported);
    let ds=model.bases.iter().find(|s|s.notion_id==DS1).unwrap();
    assert_eq!(model.rows_for_base(&ds.akula_id).count(),2);
    assert_eq!(model.views_for_base(&ds.akula_id).count(),2);
    assert_eq!(ds.columns.len(),5);
    assert!(ds.columns.iter().all(|c|!c.native_edit_supported));
    let formula=ds.columns.iter().find(|c|c.source_type=="formula").unwrap();
    assert_eq!(formula.source_definition.pointer("/formula/expression")
        .and_then(Value::as_str),Some("prop(\"Score\") + 1"));
    let row=find(&model,PAGE_B);
    assert_eq!(row.source_revision.as_deref(),Some("2026-10-24T21:30:00Z"));
    assert_eq!(cell(row,"number%3Ascore").binding,Binding::Exact);
    assert_eq!(cell(row,"number%3Ascore").source_value["number"],json!(42));
    assert_eq!(cell(row,"formula").binding,Binding::Exact);
    assert_eq!(cell(row,"formula").source_value["formula"]["number"],json!(43));
    assert!(!cell(row,"formula").native_edit_supported,
        "formula cache is a static result, not a calculation engine");
    assert_eq!(model.views_for_base(&ds.akula_id).map(|v|v.view_type.as_str())
        .collect::<Vec<_>>(),vec!["table","board"].into_iter().collect::<Vec<_>>());
    assert!(model.views.iter().all(|v|!v.native_render_supported));
    assert!(model.views.iter().all(|v|v.source_configuration.get("filter").is_some()));
}
#[test]
fn rename_and_reorder_preserve_object_and_property_identity_without_row_duplication() {
    let original=compose(&fixture(),DIGEST).unwrap();
    let mut changed=fixture();
    changed.objects.reverse();
    for object in &mut changed.objects {
        if object.kind=="page" {
            if object.json.get("properties").is_some() {
                object.json["properties"]["Name"]["title"]=json!([{"plain_text":"Renamed"}]);
            }
        } else if object.kind=="data_source" {
            object.json["name"]=json!("New display name");
        }
    }
    let next=compose(&changed,DIGEST).unwrap();
    assert_eq!(original.rows.len(),next.rows.len());
    assert_eq!(original.views.len(),next.views.len());
    for id in [ROOT,PAGE_B,PAGE_C,PAGE_D] {
        assert_eq!(find(&original,id).akula_id,find(&next,id).akula_id);
    }
    assert_eq!(original.bases.iter().map(|s|&s.akula_id).collect::<Vec<_>>(),
        next.bases.iter().map(|s|&s.akula_id).collect::<Vec<_>>());
    assert_eq!(original.bases.iter().map(|s|s.columns.iter()
        .map(|c|&c.akula_property_id).collect::<Vec<_>>()).collect::<Vec<_>>(),
        next.bases.iter().map(|s|s.columns.iter()
        .map(|c|&c.akula_property_id).collect::<Vec<_>>()).collect::<Vec<_>>());
    assert_eq!(original.views.iter().map(|v|&v.akula_id).collect::<Vec<_>>(),
        next.views.iter().map(|v|&v.akula_id).collect::<Vec<_>>());
    assert_ne!(node_id(ROOT,"page",PAGE_B),
        node_id(PAGE_B,"page",PAGE_B),"different roots cannot be merged");
}
#[test]
fn unmatched_missing_and_type_changed_fields_are_explicitly_reviewable() {
    let mut source=fixture();
    source.objects.retain(|o|!(o.kind=="data_source"&&o.id==DS2));
    let first=source.objects.iter_mut().find(|o|o.kind=="data_source").unwrap();
    first.json["properties"]["Missing ID"]=json!({"type":"url"});
    let page=source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==PAGE_B).unwrap();
    page.json["properties"]["Score"]=json!({
        "id":"number%3Ascore","type":"rich_text","rich_text":[{"plain_text":"oops"}]
    });
    page.json["properties"]["Unnamed"]=json!({"type":"status","status":{"name":"New"}});
    page.json["properties"]["New field"]=json!({
        "id":"new-property","type":"checkbox","checkbox":true
    });
    let model=compose(&source,DIGEST).unwrap();
    assert!(model.needs_review());
    assert_eq!(model.unbound_rows,2);
    assert_eq!(model.unbound_views,0);
    assert!(model.fields_needing_review>=4);
    let row=find(&model,PAGE_B);
    assert_eq!(cell(row,"number%3Ascore").binding,Binding::TypeMismatch);
    assert_eq!(cell(row,"number%3Ascore").akula_property_id,None);
    assert_eq!(cell(row,"new-property").binding,Binding::UnknownProperty);
    assert!(row.cells.iter().any(|c|c.binding==Binding::NoPropertyId));
    assert!(find(&model,PAGE_C).akula_base_id.is_none());
    assert_eq!(find(&model,PAGE_C).cells.iter()
        .filter(|c|c.binding==Binding::NoSchema).count(),5);
}
#[test]
fn duplicate_schema_property_ids_are_rejected_without_fabrication() {
    let mut source=fixture();
    let base=source.objects.iter_mut().find(|o|o.kind=="data_source").unwrap();
    base.json["properties"]["Duplicate ID"]=json!({"id":"title","type":"select"});
    assert!(compose(&source,DIGEST).is_err());
}
#[test]
fn verified_sqlite_is_read_only_and_rejects_stale_digest() {
    let tmp=tempfile::tempdir().unwrap();
    let db=tmp.path().join("ui-state.sqlite3");
    let missing=from_verified_cache(&db,ROOT,DIGEST);
    assert!(missing.is_err());
    assert!(!db.exists(),"read-only model lookup must not create cache");
    let digest={
        let mut store=SnapshotStore::open(&db).unwrap();
        store.publish(&fixture(),123).unwrap().digest
    };
    let model=from_verified_cache(&db,ROOT,&digest).unwrap();
    assert_eq!(model.bases.len(),2);
    assert_eq!(model.rows.len(),4);
    assert_eq!(model.views.len(),2);
    assert_eq!(model.source_digest,digest);
    assert!(from_verified_cache(&db,ROOT,DIGEST).is_err());
}

#[test]
fn independently_owned_bases_survive_token_and_cache_loss() {
    use crate::{
        app_core::{Core, CoreEvent},
        notion_api_probe::{projection::prepare_import, publish, structured_bundle},
    };
    use std::{fs, sync::Arc};
    let source=fixture();
    let temp=tempfile::tempdir().unwrap();
    let app=temp.path().join("app");
    let vault=temp.path().join("vault");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&vault).unwrap();
    let db=app.join("ui-state.sqlite3");
    let digest={
        let mut store=SnapshotStore::open(&db).unwrap();
        store.publish(&source,77).unwrap().digest
    };
    let checked=structured_bundle::prepare_from_cached(&db,ROOT,&digest).unwrap();
    let notes=prepare_import(&source).unwrap();
    assert!(notes.publish_allowed);
    let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(),1).unwrap();
    assert!(publish::publish_approved(&core,&vault,&notes,true).unwrap().completed);
    let receipt=structured_bundle::publish_approved(&core,&vault,&checked,true).unwrap();
    let path=vault.join(receipt.relative_path);
    let before=from_independent_bundle(&path).unwrap();
    assert_eq!(before.bases.len(),2);
    assert_eq!(before.rows.len(),4);
    assert_eq!(before.views.len(),2);
    assert_eq!(before.source_digest,digest);
    assert!(!before.native_edit_supported);
    let _=fs::remove_file(&db);
    let after=from_independent_bundle(&path).unwrap();
    assert_eq!(after.bases.len(),before.bases.len());
    assert_eq!(after.rows.len(),before.rows.len());
    assert_eq!(after.views.len(),before.views.len());
    assert_eq!(after.source_digest,before.source_digest);
    fs::write(&path,b"local modification").unwrap();
    assert!(from_independent_bundle(&path).is_err());
}
#[test]
fn modern_database_container_does_not_create_an_empty_duplicate_schema() {
    let mut source=fixture();
    source.objects.push(RawObject{
        kind:"database".into(),
        id:"66666666-6666-4666-8666-666666666666".into(),
        json:json!({"object":"database",
            "data_sources":[{"id":DS1},{"id":DS2}]}),
    });
    let model=compose(&source,DIGEST).unwrap();
    assert_eq!(model.bases.len(),2,"data sources carry schema; database is a wrapper");
    assert_eq!(model.rows.len(),4);
    assert_eq!(model.fields_needing_review,0);
}
