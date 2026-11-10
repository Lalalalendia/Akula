use super::*;
use crate::{
    app_core::{Core, CoreEvent},
    notion_api_probe::{projection::prepare_import, publish, SourcePreview, RawObject, API_VERSION},
};
use serde_json::json;
use std::sync::Arc;

const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const C: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const D: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const DS1: &str = "11111111-1111-4111-8111-111111111111";
const DS2: &str = "22222222-2222-4222-8222-222222222222";
const V1: &str = "33333333-3333-4333-8333-333333333333";
const V2: &str = "44444444-4444-4444-8444-444444444444";
const PROPERTY_ID: &str = "relation%3Aptr";
const DIGEST: &str = "abcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd";

fn obj(kind: &str, id: &str, json: Value) -> RawObject {
    RawObject { kind:kind.to_owned(), id:id.to_owned(), json }
}
fn schema(id: &str) -> RawObject {
    obj("data_source",id,json!({"object":"data_source","id":id,"properties":{
        "Name":{"id":"title","type":"title"},
        "Linked":{"id":PROPERTY_ID,"type":"relation","relation":{"data_source_id":DS1}},
        "Formula":{"id":"f","type":"formula","formula":{"expression":"2+3"}},
        "Total":{"id":"r","type":"rollup","rollup":{"relation_property_id":PROPERTY_ID}}
    }}))
}
fn page(id: &str, ds: &str, name: &str) -> RawObject {
    obj("page",id,json!({"object":"page","id":id,
        "parent":{"type":"data_source_id","data_source_id":ds},
        "properties":{
            "Name":{"id":"title","type":"title","title":[{"plain_text":name}]},
            "Linked":{"id":PROPERTY_ID,"type":"relation","relation":[{"id":A}]},
            "Unknown":{"type":"status","status":{"name":"Research"}}
        }}))
}
fn fixture() -> SourcePreview {
    let mut objects = vec![
        schema(DS1),schema(DS2),
        page(A,DS1,"Same title"),page(B,DS1,"Same title"),
        page(C,DS2,"Same title"),page(D,DS2,"Same title"),
        obj("view",V1,json!({"object":"view","id":V1,
            "parent":{"type":"data_source_id","data_source_id":DS1},
            "type":"table","filter":{"property":"Linked"}})),
        obj("view",V2,json!({"object":"view","id":V2,
            "parent":{"type":"data_source_id","data_source_id":DS1},
            "type":"board","group_by":{"property":"Linked"}})),
    ];
    for id in [A,B,C,D] {
        objects.push(obj("markdown",id,json!({"markdown":"PRIVATE-SOURCE-CONTENT"})));
    }
    SourcePreview {
        format:"synthetic_root_registry",
        api_version:API_VERSION,root:A.to_owned(),
        workspace_complete:false,root_traversal_complete:true,requests:12,
        objects,links:vec![
            Link{from:B.into(),to:A.into(),kind:"relation".into(),
                property_id:Some(PROPERTY_ID.into())},
            Link{from:C.into(),to:D.into(),kind:"relation".into(),
                property_id:Some(PROPERTY_ID.into())},
        ],unresolved_target_ids:vec![],warnings:vec![],
    }
}

#[test]
fn stable_ids_are_independent_of_names_order_and_notions_view_count() {
    let source=fixture();
    let first=normalize(&source,DIGEST).unwrap();
    assert_eq!(first.review().node_count,8); // 2 sources, 4 rows, 2 views
    assert_eq!(first.review().schema_property_count,8);
    assert_eq!(first.review().row_property_count,12);
    assert_eq!(first.review().unmatched_row_property_count,4);
    assert_eq!(first.review().unresolved_edge_count,0);
    assert_eq!(first.review().resolved_edge_count,8); // 4 parents, 2 view parents, 2 relations
    assert!(!first.review().workspace_identity_verified);
    assert!(!first.review().cross_root_merge_supported);
    assert!(!first.review().editable_bases_restored);
    let envelope: Envelope=serde_json::from_slice(&first.bytes).unwrap();
    let props=&envelope.payload.schema_properties;
    assert_eq!(props.iter().filter(|p|p.source_property_id==PROPERTY_ID).count(),2);
    assert_ne!(props.iter().find(|p|p.owner_id==DS1&&p.source_property_id==PROPERTY_ID).unwrap().akula_id,
        props.iter().find(|p|p.owner_id==DS2&&p.source_property_id==PROPERTY_ID).unwrap().akula_id);
    let mut reordered=fixture();
    reordered.objects.reverse();
    reordered.links.reverse();
    for node in &mut reordered.objects {
        if node.kind=="page" {
            node.json["properties"]["Name"]["title"]=json!([{"plain_text":"Renamed"}]);
        }
    }
    let second=normalize(&reordered,DIGEST).unwrap();
    assert_eq!(first.bytes,second.bytes,"rename and object order may not change stable identity");
    let raw=String::from_utf8(first.bytes).unwrap();
    assert!(!raw.contains("Same title"));
    assert!(!raw.contains("PRIVATE-SOURCE-CONTENT"));
    assert!(!raw.contains("2+3"));
}

#[test]
fn missing_and_ambiguous_targets_never_create_invented_reverse_links() {
    let mut source=fixture();
    source.links.push(Link {
        from:A.into(),to:"55555555-5555-4555-8555-555555555555".into(),
        kind:"relation".into(),property_id:Some(PROPERTY_ID.into()),
    });
    source.objects.push(obj("block",B,json!({"object":"block","id":B})));
    let result=normalize(&source,DIGEST).unwrap();
    // Duplicate B identity makes BOTH its relation and its parent edge
    // ambiguous; the missing external target is a third unresolved edge.
    assert_eq!(result.review().unresolved_edge_count,3);
    let payload:Envelope=serde_json::from_slice(&result.bytes).unwrap();
    assert_eq!(payload.payload.edges.iter().filter(|e|e.from==B
        &&e.resolution==Resolution::Ambiguous).count(),2);
    let ambiguous=payload.payload.edges.iter().find(|e|
        e.from==B&&e.to==A&&e.kind=="relation").unwrap();
    assert_eq!(ambiguous.resolution,Resolution::Ambiguous);
    assert!(ambiguous.from_akula_id.is_none());
    let missing=payload.payload.edges.iter().find(|e|e.from==A
        &&e.to.starts_with("55555555")).unwrap();
    assert_eq!(missing.resolution,Resolution::Missing);
    assert!(missing.to_akula_id.is_none());
    assert!(!payload.payload.edges.iter().any(|e|e.from==A&&e.to==B
        &&e.kind=="relation"),"must not invent a bidirectional relation");
}

#[test]
fn thirty_distinct_relations_are_not_truncated_to_notion_default_page_size() {
    let mut source=fixture();
    let mut first_target=String::new();
    for i in 0..30 {
        let id=format!("00000000-0000-4000-8000-{i:012x}");
        if i==0 {first_target=id.clone();}
        source.objects.push(obj("page",&id,json!({"object":"page","id":id})));
        source.links.push(Link{from:A.into(),to:id,kind:"relation".into(),
            property_id:Some(PROPERTY_ID.into())});
    }
    source.links.push(Link{from:A.into(),to:first_target,kind:"relation".into(),
        property_id:Some(PROPERTY_ID.into())});
    let registry=normalize(&source,DIGEST).unwrap();
    assert_eq!(registry.review().resolved_edge_count,38);
    assert_eq!(registry.review().duplicate_source_edges,1);
    assert_eq!(registry.review().unresolved_edge_count,0);
    assert_eq!(registry.review().node_count,38);
}

#[test]
fn verified_cache_registry_is_independent_of_token_with_create_only_gate() {
    let temp=tempfile::tempdir().unwrap();
    let app=temp.path().join("app");
    let vault=temp.path().join("vault");
    fs::create_dir(&app).unwrap();
    fs::create_dir(&vault).unwrap();
    let db=app.join("ui-state.sqlite3");
    let digest={
        let mut store=SnapshotStore::open(&db).unwrap();
        store.publish(&fixture(),111).unwrap().digest
    };
    let registry=prepare_from_cached(&db,A,&digest).unwrap();
    assert!(publish_approved(&Core::open(&app,Arc::new(|_:CoreEvent|{})),
        &vault,&registry,false).is_err());
    assert!(!vault.join(DIR).exists());

    let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(),1).unwrap();
    assert!(publish_approved(&core,&vault,&registry,true).is_err(),
        "registry cannot create a folder or bypass approved note import");
    let preview=prepare_import(&fixture()).unwrap();
    let note_receipt=publish::publish_approved(&core,&vault,&preview,true).unwrap();
    assert!(note_receipt.completed);
    let written=publish_approved(&core,&vault,&registry,true).unwrap();
    assert!(!written.already_identical);
    let path=vault.join(&written.relative_path);
    assert_eq!(inspect_existing(&path).unwrap().payload_blake3,
        registry.review().payload_blake3);
    let repeated=publish_approved(&core,&vault,&registry,true).unwrap();
    assert!(repeated.already_identical);
    fs::remove_file(&db).ok();
    assert_eq!(inspect_existing(&path).unwrap().node_count,8);
    fs::write(&path,b"local changes").unwrap();
    assert!(publish_approved(&core,&vault,&registry,true).is_err());
    assert_eq!(fs::read(&path).unwrap(),b"local changes");
}

#[test]
fn stale_digest_and_tampered_registry_fail_closed() {
    let temp=tempfile::tempdir().unwrap();
    let db=temp.path().join("ui-state.sqlite3");
    let initial={
        let mut store=SnapshotStore::open(&db).unwrap();
        store.publish(&fixture(),1).unwrap().digest
    };
    assert!(prepare_from_cached(&db,A,&"0".repeat(64)).is_err());
    assert!(prepare_from_cached(&db,B,&initial).is_err());
    let expected=prepare_from_cached(&db,A,&initial).unwrap();
    let mut corrupted=expected.bytes.clone();
    let at=corrupted.iter().position(|&byte|byte==b'a').unwrap();
    corrupted[at]=b'b';
    assert!(verify_bytes(&corrupted).is_err());
    let mut changed=fixture();
    changed.objects.push(obj("view",
        "66666666-6666-4666-8666-666666666666",
        json!({"object":"view","id":"66666666-6666-4666-8666-666666666666"})));
    let mut store=SnapshotStore::open(&db).unwrap();
    store.publish(&changed,2).unwrap();
    assert!(prepare_from_cached(&db,A,&initial).is_err());
}

#[test]
fn missing_schema_ids_and_api_unresolved_targets_are_explicitly_partial() {
    let mut source=fixture();
    let schema=source.objects.iter_mut()
        .find(|o|o.kind=="data_source"&&o.id==DS1).unwrap();
    schema.json["properties"]["Formula"]
        .as_object_mut().unwrap().remove("id");
    source.unresolved_target_ids.push("55555555-5555-4555-8555-555555555555".into());
    let prepared=normalize(&source,DIGEST).unwrap();
    assert_eq!(prepared.review().unidentified_schema_property_count,1);
    assert_eq!(prepared.review().unresolved_source_target_count,1);
    assert!(!prepared.review().editable_bases_restored);
    let payload:Envelope=serde_json::from_slice(&prepared.bytes).unwrap();
    assert!(payload.payload.schema_properties.iter().all(|p|
        !(p.owner_id==DS1&&p.source_property_id=="f")));
}
