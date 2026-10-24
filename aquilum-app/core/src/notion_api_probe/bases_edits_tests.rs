use super::*;
use crate::{
    app_core::{Core, CoreEvent},
    notion_api_probe::{
        bases_model, projection::prepare_import, publish,
        snapshot::SnapshotStore, structured_bundle, Link, RawObject,
        SourcePreview, API_VERSION,
    },
};
use serde_json::{json, Value};
use std::{fs, sync::Arc};

const A:&str="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const B:&str="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const C:&str="cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const DS1:&str="11111111-1111-4111-8111-111111111111";
const DS2:&str="22222222-2222-4222-8222-222222222222";
const BAD:&str="99999999-9999-4999-8999-999999999999";

fn obj(kind:&str,id:&str,json:Value)->RawObject {
    RawObject{kind:kind.into(),id:id.into(),json}
}
fn data_source(id:&str)->RawObject {
    obj("data_source",id,json!({"object":"data_source","id":id,
        "properties":{
            "Name":{"id":"title","type":"title"},
            "Score":{"id":"score","type":"number"},
            "Done":{"id":"done","type":"checkbox"},
            "Related":{"id":"rel","type":"relation",
                "relation":{"data_source_id":DS2}},
            "Calculated":{"id":"calc","type":"formula",
                "formula":{"expression":"prop(\"Score\") + 1"}}
        }}))
}
fn page(id:&str,ds:&str,score:i64)->RawObject {
    obj("page",id,json!({"object":"page","id":id,
        "parent":{"type":"data_source_id","data_source_id":ds},
        "properties":{
            "Name":{"id":"title","type":"title",
                "title":[{"plain_text":"Same name"}]},
            "Score":{"id":"score","type":"number","number":score},
            "Done":{"id":"done","type":"checkbox","checkbox":false},
            "Related":{"id":"rel","type":"relation",
                "relation":[{"id":B}]},
            "Calculated":{"id":"calc","type":"formula",
                "formula":{"type":"number","number":score+1}}
        }}))
}
fn fixture()->SourcePreview{
    let mut objects=vec![
        data_source(DS1),data_source(DS2),
        page(A,DS1,5),page(B,DS2,17),page(C,DS1,99),
    ];
    for id in [A,B,C] {
        objects.push(obj("markdown",id,json!({"markdown":format!("Body {id}")})));
    }
    SourcePreview{
        format:"synthetic_approved_api_root",api_version:API_VERSION,root:A.into(),
        workspace_complete:false,root_traversal_complete:true,requests:8,
        objects,links:vec![Link{from:A.into(),to:B.into(),kind:"relation".into(),
            property_id:Some("rel".into())}],
        unresolved_target_ids:vec![],warnings:vec![],
    }
}
struct Fixture {
    _temp:tempfile::TempDir,
    core:Arc<Core>,
    app:PathBuf,
    vault:PathBuf,
    digest:String,
    bundle:String,
}
impl Fixture {
    fn new()->Self {
        let tmp=tempfile::tempdir().unwrap();
        let app=tmp.path().join("app");
        let vault=tmp.path().join("vault");
        fs::create_dir(&app).unwrap();
        fs::create_dir(&vault).unwrap();
        let db=app.join("ui-state.sqlite3");
        let digest={
            let mut store=SnapshotStore::open(&db).unwrap();
            store.publish(&fixture(),25).unwrap().digest
        };
        let bundle=structured_bundle::prepare_from_cached(&db,A,&digest).unwrap();
        let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(),1).unwrap();
        let notes=prepare_import(&fixture()).unwrap();
        assert!(notes.publish_allowed);
        assert!(publish::publish_approved(&core,&vault,&notes,true).unwrap().completed);
        let receipt=structured_bundle::publish_approved(&core,&vault,&bundle,true).unwrap();
        Self{_temp:tmp,core,app,vault,digest,bundle:receipt.relative_path}
    }
    fn model(&self)->BasesModel{
        bases_model::from_independent_bundle(&self.vault.join(&self.bundle)).unwrap()
    }
    fn row(&self,id:&str)->String{
        self.model().rows.iter().find(|r|r.notion_page_id==id).unwrap().akula_id.clone()
    }
    fn prop(&self,ds:&str,id:&str)->String{
        self.model().bases.iter().find(|s|s.notion_id==ds).unwrap().columns.iter()
            .find(|c|c.notion_property_id.as_deref()==Some(id)).unwrap()
            .akula_property_id.clone().unwrap()
    }
    fn edited_path(&self)->PathBuf{self.vault.join(relative(A))}
}
#[test]
fn approved_atomic_multicell_revision_is_independent_and_stale_cas_refused() {
    let f=Fixture::new();
    let original=fs::read(f.vault.join(&f.bundle)).unwrap();
    let score=f.prop(DS1,"score");
    let rel=f.prop(DS1,"rel");
    let row=f.row(A);
    let b=f.row(B);
    let ops=vec![
        Change::Set{row_id:row.clone(),property_id:score.clone(),
            value:LocalValue::Number(42.5)},
        Change::Set{row_id:row.clone(),property_id:rel.clone(),
            value:LocalValue::Relation(vec![b.clone()])},
    ];
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,&ops,false).is_err());
    assert!(!f.edited_path().exists());
    let first=apply_approved(&f.core,&f.vault,A,&f.digest,None,&ops,true).unwrap();
    assert!(first.created&&first.source_unchanged);
    assert_eq!(first.revision,1);
    assert_eq!(first.overrides,2);
    assert_eq!(fs::read(f.vault.join(&f.bundle)).unwrap(),original);
    let opened=open(&f.vault,A,&f.digest).unwrap().unwrap();
    assert_eq!(opened.state.local_value(&row,&score),Some(&LocalValue::Number(42.5)));
    assert_eq!(opened.state.local_value(&row,&rel),
        Some(&LocalValue::Relation(vec![b])));
    assert_eq!(opened.expected_disk_hash,first.disk_hash);
    let next=apply_approved(&f.core,&f.vault,A,&f.digest,
        Some(&first.disk_hash),
        &[Change::Set{row_id:row.clone(),property_id:score.clone(),
            value:LocalValue::Number(50.0)}],true).unwrap();
    assert_eq!(next.revision,2);
    assert!(!next.created);
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,Some(&first.disk_hash),
        &[Change::Revert{row_id:row,property_id:score}],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
}
#[test]
fn invalid_types_missing_targets_and_calculated_values_never_publish() {
    let f=Fixture::new();
    let row=f.row(A);
    let checks=vec![
        (f.prop(DS1,"score"),LocalValue::Text("number?".into())),
        (f.prop(DS1,"calc"),LocalValue::Number(22.0)),
        (f.prop(DS1,"rel"),LocalValue::Relation(vec![f.row(C)])),
        (f.prop(DS1,"rel"),LocalValue::Relation(vec![BAD.into()])),
        (f.prop(DS1,"done"),LocalValue::Number(f64::INFINITY)),
    ];
    for (property_id,value) in checks {
        assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
            &[Change::Set{row_id:row.clone(),property_id,value}],true).is_err());
        assert!(!f.edited_path().exists());
    }
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:f.prop(DS1,"score"),
            value:LocalValue::Number(10.0)},
          Change::Set{row_id:row,property_id:f.prop(DS1,"calc"),
            value:LocalValue::Number(11.0)}],true).is_err());
    assert!(!f.edited_path().exists(),"invalid second command rolls back full batch");
}
#[test]
fn local_revert_exposes_original_without_deleting_source_field() {
    let f=Fixture::new();
    let row=f.row(A);
    let prop=f.prop(DS1,"done");
    let created=apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Checkbox(true)}],true).unwrap();
    let reverted=apply_approved(&f.core,&f.vault,A,&f.digest,Some(&created.disk_hash),
        &[Change::Revert{row_id:row.clone(),property_id:prop.clone()}],true).unwrap();
    assert_eq!(reverted.revision,2);
    assert_eq!(reverted.overrides,0);
    let opened=open(&f.vault,A,&f.digest).unwrap().unwrap();
    assert_eq!(opened.state.local_value(&row,&prop),None);
    assert_eq!(f.model().rows.iter().find(|r|r.akula_id==row).unwrap().cells.iter()
        .find(|c|c.akula_property_id.as_deref()==Some(&prop))
        .unwrap().source_value["checkbox"],json!(false));
}
#[test]
fn new_notion_digest_keeps_local_edits_and_requires_explicit_rebase() {
    let f=Fixture::new();
    let row=f.row(A);
    let prop=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row,property_id:prop,value:LocalValue::Number(777.0)}],
        true).unwrap();
    let before=fs::read(f.edited_path()).unwrap();
    let mut new_source=fixture();
    new_source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
        .unwrap().json["properties"]["Score"]["number"]=json!(91);
    let db=f.app.join("ui-state.sqlite3");
    let new_digest={
        let mut writer=SnapshotStore::open(&db).unwrap();
        writer.publish(&new_source,26).unwrap().digest
    };
    assert_ne!(new_digest,f.digest);
    let newer=structured_bundle::prepare_from_cached(&db,A,&new_digest).unwrap();
    structured_bundle::publish_approved(&f.core,&f.vault,&newer,true).unwrap();
    assert!(open(&f.vault,A,&new_digest).is_err(),
        "new source may not silently adopt old local overrides");
    assert!(apply_approved(&f.core,&f.vault,A,&new_digest,None,
        &[Change::Set{row_id:f.row(A),property_id:f.prop(DS1,"score"),
            value:LocalValue::Number(1.0)}],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
}
#[test]
fn corrupt_existing_file_is_not_replaced_or_treated_as_new() {
    let f=Fixture::new();
    fs::write(f.edited_path(),b"untrusted external content").unwrap();
    let bytes=fs::read(f.edited_path()).unwrap();
    assert!(open(&f.vault,A,&f.digest).is_err());
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:f.row(A),property_id:f.prop(DS1,"score"),
            value:LocalValue::Number(12.0)}],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),bytes);
}
