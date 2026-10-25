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

#[test]
fn local_columns_have_stable_ids_and_project_effective_values() {
    let f=Fixture::new();
    let original=fs::read(f.vault.join(&f.bundle)).unwrap();
    let base=f.model().bases.iter().find(|s|s.notion_id==DS1).unwrap().akula_id.clone();
    let row_a=f.row(A);
    let row_c=f.row(C);
    let local_uuid="55555555-5555-4555-8555-555555555555";
    let local_id=local_property(A,&base,local_uuid);
    let score=f.prop(DS1,"score");
    let formula=f.prop(DS1,"calc");
    let batch=vec![
        Change::AddColumn{base_id:base.clone(),local_id:local_uuid.into(),
            name:"Priority note".into(),kind:LocalKind::Text},
        Change::Set{row_id:row_a.clone(),property_id:local_id.clone(),
            value:LocalValue::Text("Local only".into())},
        Change::Set{row_id:row_a.clone(),property_id:score.clone(),
            value:LocalValue::Number(34.0)},
    ];
    let first=apply_approved(&f.core,&f.vault,A,&f.digest,None,&batch,true).unwrap();
    assert_eq!(first.revision,1);
    assert_eq!(first.local_columns,1);
    assert_eq!(first.overrides,2);
    assert_eq!(fs::read(f.vault.join(&f.bundle)).unwrap(),original);
    let effective=read_effective_base(&f.vault,A,&f.digest,&base).unwrap();
    assert_eq!(effective.rows.len(),2,"multiple views never duplicate rows");
    assert_eq!(effective.local_revision,1);
    assert!(!effective.native_views_supported);
    assert!(effective.columns.iter().any(|c|c.property_id==local_id
        &&c.local_only&&c.editable_locally&&c.name=="Priority note"));
    assert!(effective.columns.iter().any(|c|c.property_id==formula
        &&!c.editable_locally));
    let row=effective.rows.iter().find(|r|r.akula_id==row_a).unwrap();
    assert_eq!(row.cells.iter().find(|c|c.property_id==local_id).unwrap().value,
        EffectiveValue::Local(LocalValue::Text("Local only".into())));
    assert_eq!(row.cells.iter().find(|c|c.property_id==score).unwrap().value,
        EffectiveValue::Local(LocalValue::Number(34.0)));
    assert!(matches!(row.cells.iter().find(|c|c.property_id==formula).unwrap().value,
        EffectiveValue::Source(_)),"formula remains raw inert Notion JSON");
    let other=effective.rows.iter().find(|r|r.akula_id==row_c).unwrap();
    assert_eq!(other.cells.iter().find(|c|c.property_id==local_id).unwrap().value,
        EffectiveValue::EmptyLocal);
    assert!(matches!(other.cells.iter().find(|c|c.property_id==score).unwrap().value,
        EffectiveValue::Source(_)));
    // Changes to local column display name cannot change its identity.
    let second=apply_approved(&f.core,&f.vault,A,&f.digest,Some(&first.disk_hash),
        &[Change::RenameLocalColumn{property_id:local_id.clone(),
            name:"Private notes".into()}],true).unwrap();
    assert_eq!(second.revision,2);
    assert_eq!(second.local_columns,1);
    // A physically separate portable Vault has no appdata/cache or token.
    // Removing an open SQLite file is not reliable on Windows; copying only
    // the autonomous bundle and overlay is a stronger offline assertion.
    let portable=tempfile::tempdir().unwrap();
    fs::create_dir(portable.path().join("Notion-import")).unwrap();
    fs::copy(f.vault.join(&f.bundle),portable.path().join(&f.bundle)).unwrap();
    fs::copy(f.edited_path(),portable.path().join(relative(A))).unwrap();
    assert!(!portable.path().join("ui-state.sqlite3").exists());
    let offline=read_effective_base(portable.path(),A,&f.digest,&base).unwrap();
    assert_eq!(offline.local_revision,2);
    assert!(offline.columns.iter().any(|c|c.property_id==local_id
        &&c.name=="Private notes"));
    assert_eq!(offline.rows.iter().find(|r|r.akula_id==row_a).unwrap()
        .cells.iter().find(|c|c.property_id==local_id).unwrap().value,
        EffectiveValue::Local(LocalValue::Text("Local only".into())));
    assert_eq!(fs::read(f.vault.join(&f.bundle)).unwrap(),original);
}
#[test]
fn schema_name_collisions_duplicate_local_ids_and_source_rename_fail_atomically() {
    let f=Fixture::new();
    let base=f.model().bases.iter().find(|s|s.notion_id==DS1).unwrap().akula_id.clone();
    let uuid="66666666-6666-4666-8666-666666666666";
    let make=|name:&str|Change::AddColumn{
        base_id:base.clone(),local_id:uuid.into(),name:name.into(),kind:LocalKind::Text,
    };
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[make("name")],true).is_err(),"source Name column collision");
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[make("Scratch"),make("Scratch 2")],true).is_err(),"same local UUID");
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[make("Scratch"),Change::AddColumn{
            base_id:base.clone(),local_id:"77777777-7777-4777-8777-777777777777".into(),
            name:"SCRATCH".into(),kind:LocalKind::Number}],true).is_err());
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::RenameLocalColumn{property_id:f.prop(DS1,"score"),
            name:"Changed by local overlay".into()}],true).is_err());
    assert!(!f.edited_path().exists(),"invalid batch must create no file");
    let ok=apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[make("Scratch")],true).unwrap();
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,Some(&ok.disk_hash),
        &[Change::RenameLocalColumn{
            property_id:local_property(A,&base,uuid),name:"Name".into()}],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
}
#[test]
fn local_column_type_checks_fail_entire_batch_before_publication() {
    let f=Fixture::new();
    let base=f.model().bases.iter().find(|s|s.notion_id==DS1).unwrap().akula_id.clone();
    let uuid="88888888-8888-4888-8888-888888888888";
    let prop=local_property(A,&base,uuid);
    let add=Change::AddColumn{base_id:base.clone(),local_id:uuid.into(),
        name:"Local number".into(),kind:LocalKind::Number};
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[add,Change::Set{row_id:f.row(A),property_id:prop,
            value:LocalValue::Text("wrong type".into())}],true).is_err());
    assert!(!f.edited_path().exists());
    let unknown=Change::AddColumn{
        base_id:"unknown-source".into(),local_id:uuid.into(),
        name:"Misplaced".into(),kind:LocalKind::Checkbox};
    assert!(apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[unknown],true).is_err());
    assert!(!f.edited_path().exists());
}
#[test]
fn existing_v1_overlay_without_local_columns_remains_readable() {
    let f=Fixture::new();
    let row=f.row(A);
    let score=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:score.clone(),
            value:LocalValue::Number(25.0)}],true).unwrap();
    let mut legacy:Value=serde_json::from_slice(&fs::read(f.edited_path()).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("local_columns");
    fs::write(f.edited_path(),serde_json::to_vec(&legacy).unwrap()).unwrap();
    let state=open(&f.vault,A,&f.digest).unwrap().unwrap();
    assert!(state.state.local_columns.is_empty());
    assert_eq!(state.state.local_value(&row,&score),Some(&LocalValue::Number(25.0)));
    let next=apply_approved(&f.core,&f.vault,A,&f.digest,Some(&state.expected_disk_hash),
        &[Change::Set{row_id:row,property_id:score,
            value:LocalValue::Number(26.0)}],true).unwrap();
    assert_eq!(next.revision,2);
    assert_eq!(next.local_columns,0);
}


fn published_new_source(f: &Fixture, edit: impl FnOnce(&mut SourcePreview)) -> String {
    let mut source = fixture();
    edit(&mut source);
    let db = f.app.join("ui-state.sqlite3");
    let digest = SnapshotStore::open(&db).unwrap().publish(&source, 26).unwrap().digest;
    let bundle = structured_bundle::prepare_from_cached(&db, A, &digest).unwrap();
    structured_bundle::publish_approved(&f.core, &f.vault, &bundle, true).unwrap();
    assert_ne!(digest, f.digest);
    digest
}
#[test]
fn c5b4_rebase_preview_identifies_unmodified_source_and_never_writes() {
    let f = Fixture::new();
    let row = f.row(A);
    let prop = f.prop(DS1, "score");
    let receipt = apply_approved(&f.core, &f.vault, A, &f.digest, None,
        &[Change::Set {row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(200.0)}], true).unwrap();
    let sidecar_before = fs::read(f.edited_path()).unwrap();
    let original_bundle = fs::read(f.vault.join(&f.bundle)).unwrap();
    // Another row changes, but the baseline of the edited cell does not.
    let digest = published_new_source(&f, |new| {
        new.objects.iter_mut().find(|o|o.kind=="page"&&o.id==C)
            .unwrap().json["properties"]["Score"]["number"] = json!(110);
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&digest).unwrap();
    assert!(preview.can_offer_approval());
    assert_eq!(preview.ready_cells,1);
    assert_eq!(preview.conflicted_cells,0);
    assert_eq!(preview.cells[0].disposition,RebaseDisposition::Eligible);
    assert_eq!(preview.cells[0].row_id,row);
    assert_eq!(preview.cells[0].property_id,prop);
    assert_eq!(preview.expected_local_hash,receipt.disk_hash);
    assert!(!preview.writes_performed);
    assert_eq!(fs::read(f.edited_path()).unwrap(),sidecar_before);
    assert_eq!(fs::read(f.vault.join(&f.bundle)).unwrap(),original_bundle);
    assert!(open(&f.vault,A,&digest).is_err(),
        "a preview never adopts an unapproved new snapshot");
}
#[test]
fn c5b4_rebase_preview_blocks_changes_to_the_edited_source_cell() {
    let f=Fixture::new();
    let row=f.row(A);
    let prop=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row,property_id:prop,
            value:LocalValue::Number(777.0)}],true).unwrap();
    let before=fs::read(f.edited_path()).unwrap();
    let next=published_new_source(&f, |new| {
        new.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(91);
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert_eq!(preview.ready_cells,0);
    assert_eq!(preview.conflicted_cells,1);
    assert_eq!(preview.cells[0].disposition,RebaseDisposition::SourceValueChanged);
    assert!(!preview.can_offer_approval());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
}
#[test]
fn c5b4_rebase_preview_never_silently_renames_a_colliding_local_column() {
    let f=Fixture::new();
    let base=f.model().bases.iter().find(|b|b.notion_id==DS1).unwrap().akula_id.clone();
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::AddColumn{
            base_id:base,local_id:"55555555-5555-4555-8555-555555555555".into(),
            name:"Private memo".into(),kind:LocalKind::Text,
        }],true).unwrap();
    let before=fs::read(f.edited_path()).unwrap();
    let next=published_new_source(&f, |new| {
        new.objects.iter_mut().find(|o|o.kind=="data_source"&&o.id==DS1)
            .unwrap().json["properties"]["Private memo"]=
                json!({"id":"new-field","type":"rich_text"});
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert_eq!(preview.conflicted_columns,1);
    assert_eq!(preview.ready_cells,0);
    assert!(!preview.columns[0].compatible);
    assert!(!preview.can_offer_approval());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
}
#[test]
fn c5b4_rebase_preview_blocks_same_type_schema_definition_changes() {
    let f=Fixture::new();
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:f.row(A),property_id:f.prop(DS1,"score"),
            value:LocalValue::Number(99.0)}],true).unwrap();
    let next=published_new_source(&f, |new| {
        new.objects.iter_mut().find(|o|o.kind=="data_source"&&o.id==DS1)
            .unwrap().json["properties"]["Score"]["format"]="number".into();
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert_eq!(preview.cells[0].disposition,RebaseDisposition::PropertyDefinitionChanged);
    assert!(!preview.can_offer_approval());
}


#[test]
fn c5b4_approved_rebase_preserves_local_values_only_after_exact_cas() {
    let f=Fixture::new();
    let base=f.model().bases.iter().find(|b|b.notion_id==DS1).unwrap().akula_id.clone();
    let row=f.row(A);
    let property=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:property.clone(),
            value:LocalValue::Number(700.0)}],true).unwrap();
    let original=fs::read(f.vault.join(&f.bundle)).unwrap();
    let next=published_new_source(&f, |source| {
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==C)
            .unwrap().json["properties"]["Score"]["number"]=json!(103);
    });
    let report=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert!(report.can_offer_approval());
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_approved(&f.core,&f.vault,A,&f.digest,&next,
        &report.expected_local_hash,false).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
    let receipt=apply_rebase_approved(&f.core,&f.vault,A,&f.digest,&next,
        &report.expected_local_hash,true).unwrap();
    assert_eq!(receipt.revision,2);
    assert_eq!(receipt.overrides_preserved,1);
    assert!(receipt.source_unchanged);
    assert_ne!(fs::read(f.edited_path()).unwrap(),before);
    assert_eq!(fs::read(f.vault.join(&f.bundle)).unwrap(),original);
    assert!(open(&f.vault,A,&f.digest).is_err(),
        "old baseline must not work after explicitly approved rebase");
    let active=open(&f.vault,A,&next).unwrap().unwrap();
    assert_eq!(active.state.baseline_digest,next);
    assert_eq!(active.expected_disk_hash,receipt.disk_hash);
    let effective=read_effective_base(&f.vault,A,&next,&base).unwrap();
    let selected=effective.rows.iter().find(|r|r.akula_id==row).unwrap();
    assert_eq!(selected.cells.iter().find(|c|c.property_id==property).unwrap().value,
        EffectiveValue::Local(LocalValue::Number(700.0)));
}
#[test]
fn c5b4_approved_rebase_denies_changed_source_and_stale_hash() {
    let f=Fixture::new();
    let row=f.row(A);let property=f.prop(DS1,"score");
    let initial=apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:property.clone(),
            value:LocalValue::Number(3.0)}],true).unwrap();
    let next=published_new_source(&f, |source| {
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(11);
    });
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_approved(&f.core,&f.vault,A,&f.digest,&next,
        &initial.disk_hash,true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
    let compatible=published_new_source(&f, |source| {
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==C)
            .unwrap().json["properties"]["Score"]["number"]=json!(107);
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&compatible).unwrap();
    let newer=apply_approved(&f.core,&f.vault,A,&f.digest,
        Some(&initial.disk_hash),&[Change::Set{
            row_id:row,property_id:property,value:LocalValue::Number(4.0),
        }],true).unwrap();
    assert_ne!(preview.expected_local_hash,newer.disk_hash);
    let stale_before=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_approved(&f.core,&f.vault,A,&f.digest,&compatible,
        &preview.expected_local_hash,true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),stale_before);
}

#[test]
fn c5b4_keep_local_requires_exact_manual_selection_of_changed_source_values() {
    let f=Fixture::new();
    let row=f.row(A);
    let prop=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(777.0)}],true).unwrap();
    let next=published_new_source(&f, |src| {
        src.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(91);
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert_eq!(preview.cells[0].disposition,RebaseDisposition::SourceValueChanged);
    assert!(preview.cells[0].manual_review_supported);
    assert_eq!(preview.cells[0].previous_source.as_deref(),Some("5"));
    assert_eq!(preview.cells[0].updated_source.as_deref(),Some("91"));
    assert_eq!(preview.cells[0].local_override.as_deref(),Some("777"));
    let preserved=fs::read(f.edited_path()).unwrap();
    let selection=vec![(row.clone(),prop.clone())];
    for decisions in [
        vec![],
        vec![("unrelated".into(),prop.clone())],
        vec![selection[0].clone(),selection[0].clone()],
    ] {
        assert!(apply_rebase_resolved_approved(&f.core,&f.vault,A,
            &f.digest,&next,&preview.expected_local_hash,&decisions,true).is_err());
        assert_eq!(fs::read(f.edited_path()).unwrap(),preserved);
    }
    assert!(apply_rebase_resolved_approved(&f.core,&f.vault,A,&f.digest,
        &next,&preview.expected_local_hash,&selection,false).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),preserved);
    let receipt=apply_rebase_resolved_approved(&f.core,&f.vault,A,
        &f.digest,&next,&preview.expected_local_hash,&selection,true).unwrap();
    assert_eq!(receipt.approved_local_wins,1);
    let base=f.model().bases.iter().find(|b|b.notion_id==DS1).unwrap().akula_id.clone();
    let effective=read_effective_base(&f.vault,A,&next,&base).unwrap();
    assert_eq!(effective.rows.iter().find(|r|r.akula_id==row).unwrap()
        .cells.iter().find(|c|c.property_id==prop).unwrap().value,
        EffectiveValue::Local(LocalValue::Number(777.0)));
}
#[test]
fn c5b4_keep_local_rejects_property_definition_conflicts_even_when_selected() {
    let f=Fixture::new();
    let row=f.row(A);
    let property=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:property.clone(),
            value:LocalValue::Number(27.0)}],true).unwrap();
    let next=published_new_source(&f, |src| {
        src.objects.iter_mut().find(|o|o.kind=="data_source"&&o.id==DS1)
            .unwrap().json["properties"]["Score"]["format"]="percent".into();
    });
    let old=fs::read(f.edited_path()).unwrap();
    let report=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert_eq!(report.cells[0].disposition,RebaseDisposition::PropertyDefinitionChanged);
    assert!(apply_rebase_resolved_approved(&f.core,&f.vault,A,&f.digest,
        &next,&report.expected_local_hash,&[(row,property)],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),old);
}

#[test]
fn c5b4_keep_local_denies_large_unreviewable_text_and_preserves_overlay() {
    let f=Fixture::new();
    let row=f.row(A);let prop=f.prop(DS1,"title");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Text("My full local text".into())}],true).unwrap();
    let next=published_new_source(&f, |source| {
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Name"]["title"][0]["plain_text"]=
                json!("z".repeat(260));
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert_eq!(preview.cells[0].disposition,RebaseDisposition::SourceValueChanged);
    assert!(!preview.cells[0].manual_review_supported);
    assert!(preview.cells[0].updated_source.is_none());
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_resolved_approved(&f.core,&f.vault,A,&f.digest,&next,
        &preview.expected_local_hash,&[(row,prop)],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
}
#[test]
fn c5b4_keep_local_denies_metadata_only_changes_without_visible_value_diff() {
    let f=Fixture::new();
    let row=f.row(A);let prop=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(40.0)}],true).unwrap();
    let next=published_new_source(&f, |source| {
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["metadata"]="changed".into();
    });
    let preview=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    assert_eq!(preview.cells[0].disposition,RebaseDisposition::SourceValueChanged);
    assert!(!preview.cells[0].manual_review_supported);
    assert_eq!(preview.cells[0].previous_source,preview.cells[0].updated_source);
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_resolved_approved(&f.core,&f.vault,A,&f.digest,&next,
        &preview.expected_local_hash,&[(row,prop)],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
}

#[test]
fn c5b4_accept_notion_preserves_exact_old_override_archive_and_exposes_new_source() {
    let f=Fixture::new();
    let row=f.row(A);let prop=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set {row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(700.0)}],true).unwrap();
    let old_overlay=fs::read(f.edited_path()).unwrap();
    let old_source=fs::read(f.vault.join(&f.bundle)).unwrap();
    let next=published_new_source(&f,|src|{
        src.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(91);
    });
    let report=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    let choices=vec![(row.clone(),prop.clone(),RebaseDecision::AcceptNotion)];
    assert!(apply_rebase_decisions_approved(&f.core,&f.vault,A,&f.digest,
        &next,&report.expected_local_hash,&choices,false).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),old_overlay);
    let receipt=apply_rebase_decisions_approved(&f.core,&f.vault,A,&f.digest,
        &next,&report.expected_local_hash,&choices,true).unwrap();
    assert_eq!(receipt.accepted_notion_values,1);
    assert_eq!(receipt.approved_local_wins,0);
    assert_eq!(receipt.overrides_preserved,0);
    let archive=receipt.pre_rebase_archive.expect("Accept Notion needs backup");
    assert!(archive.starts_with("Notion-import/"));
    assert_eq!(fs::read(f.vault.join(&archive)).unwrap(),old_overlay);
    assert_eq!(fs::read(f.vault.join(&f.bundle)).unwrap(),old_source);
    let current=open(&f.vault,A,&next).unwrap().unwrap();
    assert!(current.state.local_value(&row,&prop).is_none());
    let base=f.model().bases.iter().find(|b|b.notion_id==DS1).unwrap().akula_id.clone();
    let effective=read_effective_base(&f.vault,A,&next,&base).unwrap();
    let cell=effective.rows.iter().find(|r|r.akula_id==row).unwrap()
        .cells.iter().find(|c|c.property_id==prop).unwrap();
    match &cell.value {
        EffectiveValue::Source(source) => {
            assert_eq!(source.get("type"),Some(&json!("number")));
            assert_eq!(source.get("number"),Some(&json!(91)));
        }
        other => panic!("expected original Notion value, got {other:?}"),
    }
}
#[test]
fn c5b4_accept_notion_rejects_old_hash_and_duplicate_choices_before_any_backup() {
    let f=Fixture::new();
    let row=f.row(A);let prop=f.prop(DS1,"score");
    let first=apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set {row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(700.0)}],true).unwrap();
    let next=published_new_source(&f,|src|{
        src.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(91);
    });
    let report=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    let choose=(row.clone(),prop.clone(),RebaseDecision::AcceptNotion);
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_decisions_approved(&f.core,&f.vault,A,&f.digest,&next,
        &report.expected_local_hash,&[choose.clone(),choose.clone()],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
    let later=apply_approved(&f.core,&f.vault,A,&f.digest,
        Some(&first.disk_hash),&[Change::Set{
            row_id:row,property_id:prop,value:LocalValue::Number(701.0),
        }],true).unwrap();
    assert_ne!(later.disk_hash,report.expected_local_hash);
    let changed=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_decisions_approved(&f.core,&f.vault,A,&f.digest,&next,
        &report.expected_local_hash,&[choose],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),changed);
}

#[test]
fn c5b4_accept_notion_refuses_existing_foreign_archive_and_preserves_local_data() {
    let f=Fixture::new();
    let row=f.row(A);let property=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:property.clone(),
            value:LocalValue::Number(66.0)}],true).unwrap();
    let next=published_new_source(&f,|src|{
        src.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(19);
    });
    let report=preview_rebase(&f.vault,A,&f.digest,&next).unwrap();
    let arch=super::rebase_archive::archive_relative(A,&f.digest,&next,
        &report.expected_local_hash);
    let archive_path=f.vault.join(&arch);
    fs::write(&archive_path,b"forged").unwrap();
    let before=fs::read(f.edited_path()).unwrap();
    assert!(apply_rebase_decisions_approved(&f.core,&f.vault,A,
        &f.digest,&next,&report.expected_local_hash,
        &[(row,property,RebaseDecision::AcceptNotion)],true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),before);
    assert_eq!(fs::read(archive_path).unwrap(),b"forged");
}

#[test]
fn c5b4_restore_one_local_cell_from_verified_archive_without_rebasing_other_data() {
    let f=Fixture::new();
    let row=f.row(A);let prop=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(777.0)}],true).unwrap();
    let before=fs::read(f.edited_path()).unwrap();
    let next=published_new_source(&f,|source|{
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(91);
    });
    let old_hash=preview_rebase(&f.vault,A,&f.digest,&next).unwrap().expected_local_hash;
    let accepted=apply_rebase_decisions_approved(&f.core,&f.vault,A,&f.digest,
        &next,&old_hash,&[(row.clone(),prop.clone(),RebaseDecision::AcceptNotion)],true).unwrap();
    let archive=accepted.pre_rebase_archive.clone().unwrap();
    assert_eq!(fs::read(f.vault.join(&archive)).unwrap(),before);
    let active_before=fs::read(f.edited_path()).unwrap();
    assert!(restore_archived_cell_approved(&f.core,&f.vault,A,
        &f.digest,&next,&old_hash,&accepted.disk_hash,
        &archive,&row,&prop,false).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),active_before);
    assert!(restore_archived_cell_approved(&f.core,&f.vault,A,
        &f.digest,&next,&old_hash,&accepted.disk_hash,
        "Notion-import/other.json",&row,&prop,true).is_err());
    assert_eq!(fs::read(f.edited_path()).unwrap(),active_before);
    let restored=restore_archived_cell_approved(&f.core,&f.vault,A,
        &f.digest,&next,&old_hash,&accepted.disk_hash,
        &archive,&row,&prop,true).unwrap();
    assert_eq!(restored.overrides,1);
    assert_eq!(restored.revision,accepted.revision+1);
    let current=open(&f.vault,A,&next).unwrap().unwrap();
    assert_eq!(current.state.local_value(&row,&prop),
        Some(&LocalValue::Number(777.0)));
    assert_eq!(fs::read(f.vault.join(&archive)).unwrap(),before);
    assert!(restore_archived_cell_approved(&f.core,&f.vault,A,
        &f.digest,&next,&old_hash,&restored.disk_hash,
        &archive,&row,&prop,true).is_err(),
        "cannot overwrite a locally edited cell with an archived value");
}

#[test]
fn c5b4_discovery_after_restart_finds_only_missing_restorable_cells() {
    let f=Fixture::new();
    let row=f.row(A);
    let prop=f.prop(DS1,"score");
    apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(711.0)}],true).unwrap();
    let next=published_new_source(&f,|source| {
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(91);
    });
    let old_hash=preview_rebase(&f.vault,A,&f.digest,&next)
        .unwrap().expected_local_hash;
    let applied=apply_rebase_decisions_approved(&f.core,&f.vault,A,
        &f.digest,&next,&old_hash,
        &[(row.clone(),prop.clone(),RebaseDecision::AcceptNotion)],true).unwrap();
    let base=f.model().bases.iter().find(|b|b.notion_id==DS1)
        .unwrap().akula_id.clone();
    let candidates=discover_restore_archives(&f.vault,A,&next,
        &applied.disk_hash,&base).unwrap();
    assert_eq!(candidates.len(),1);
    assert_eq!(candidates[0].cells.len(),1);
    assert_eq!(candidates[0].cells[0].row_id,row);
    assert_eq!(candidates[0].cells[0].property_id,prop);
    assert_eq!(candidates[0].cells[0].value_preview,"711");
    assert_eq!(candidates[0].path,applied.pre_rebase_archive.unwrap());
    let fresh=restore_archived_cell_approved(&f.core,&f.vault,A,
        &f.digest,&next,&old_hash,&applied.disk_hash,
        &candidates[0].path,&row,&prop,true).unwrap();
    assert!(discover_restore_archives(&f.vault,A,&next,
        &fresh.disk_hash,&base).unwrap().is_empty());
}
#[test]
fn c5b4_discovery_refuses_changed_active_overlay_hash() {
    let f=Fixture::new();
    let row=f.row(A);
    let prop=f.prop(DS1,"score");
    let first=apply_approved(&f.core,&f.vault,A,&f.digest,None,
        &[Change::Set{row_id:row.clone(),property_id:prop.clone(),
            value:LocalValue::Number(12.0)}],true).unwrap();
    let base=f.model().bases.iter().find(|b|b.notion_id==DS1)
        .unwrap().akula_id.clone();
    let second=apply_approved(&f.core,&f.vault,A,&f.digest,
        Some(&first.disk_hash),&[Change::Set{
            row_id:row,property_id:prop,value:LocalValue::Number(13.0),
        }],true).unwrap();
    assert_ne!(first.disk_hash,second.disk_hash);
    assert!(discover_restore_archives(&f.vault,A,&f.digest,
        &first.disk_hash,&base).is_err());
}
