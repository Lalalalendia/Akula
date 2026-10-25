//! Full native Bases worker + verified bundle + File Gate CAS lifecycle.
//! User consent is supplied explicitly at the session action boundary;
//! this is not a desktop OS dialog automation or installed release test.
use super::*;
use aquilum_core::notion_api_probe::{
    snapshot::SnapshotStore,structured_bundle,RawObject,SourcePreview,API_VERSION,
};
use serde_json::{json,Value};
use std::{fs,thread,time::Duration};

const ROOT:&str="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OTHER:&str="cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const DATA:&str="11111111-1111-4111-8111-111111111111";
fn raw(kind:&str,id:&str,value:Value)->RawObject {
    RawObject{kind:kind.into(),id:id.into(),json:value}
}
fn source(score:i64)->SourcePreview{
    let ds=raw("data_source",DATA,json!({
        "object":"data_source","id":DATA,"properties":{
            "Name":{"id":"title","type":"title"},
            "Score":{"id":"score","type":"number"}
        }
    }));
    let page=|id:&str,n:i64|raw("page",id,json!({
        "object":"page","id":id,
        "parent":{"type":"data_source_id","data_source_id":DATA},
        "properties":{
            "Name":{"id":"title","type":"title",
                "title":[{"plain_text":format!("Row {id}")}]},
            "Score":{"id":"score","type":"number","number":n}
        }
    }));
    SourcePreview{
        format:"synthetic_approved_api_root",api_version:API_VERSION,
        root:ROOT.into(),workspace_complete:false,
        root_traversal_complete:true,requests:5,
        objects:vec![
            ds,page(ROOT,score),page(OTHER,99),
            raw("markdown",ROOT,json!({"markdown":"Root text"})),
            raw("markdown",OTHER,json!({"markdown":"Other text"})),
        ],
        links:vec![],unresolved_target_ids:vec![],warnings:vec![],
    }
}
fn finish(session:&mut BasesSession){
    for _ in 0..1800 {
        session.poll();
        if !session.view().busy {return;}
        thread::sleep(Duration::from_millis(10));
    }
    panic!("native Bases worker never completed");
}
#[test]
fn native_reimport_accept_archive_cold_reopen_and_restore_via_cas() {
    let sandbox=tempfile::tempdir().unwrap();
    let app=sandbox.path().join("app");
    let vault=sandbox.path().join("vault");
    fs::create_dir(&app).unwrap();
    fs::create_dir(&vault).unwrap();
    fs::create_dir(vault.join("Notion-import")).unwrap();
    let db=app.join("ui-state.sqlite3");
    let old=SnapshotStore::open(&db).unwrap()
        .publish(&source(5),25).unwrap().digest;
    let first=structured_bundle::prepare_from_cached(&db,ROOT,&old).unwrap();
    let core=Core::open(&app,Arc::new(|_:aquilum_core::CoreEvent|{}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(),1).unwrap();
    let old_bundle=structured_bundle::publish_approved(&core,&vault,&first,true)
        .unwrap().relative_path;
    let model=bases_model::from_independent_bundle(&vault.join(&old_bundle)).unwrap();
    let base=model.bases.iter().find(|b|b.notion_id==DATA).unwrap().akula_id.clone();
    let row=model.rows.iter().find(|r|r.notion_page_id==ROOT).unwrap().akula_id.clone();
    let property=model.bases.iter().find(|b|b.notion_id==DATA).unwrap()
        .columns.iter().find(|c|c.notion_property_id.as_deref()==Some("score"))
        .unwrap().akula_property_id.clone().unwrap();
    let local=bases_edits::apply_approved(&core,&vault,ROOT,&old,None,
        &[Change::Set{
            row_id:row.clone(),property_id:property.clone(),
            value:LocalValue::Number(721.0),
        }],true).unwrap();
    let local_before=fs::read(vault.join(format!("Notion-import/{ROOT}.akula-bases-edits.json"))).unwrap();
    // Validate old bundle and overlay independently of the GUI.
    assert_eq!(local.revision,1);
    let old_bytes=fs::read(vault.join(&old_bundle)).unwrap();
    let new=SnapshotStore::open(&db).unwrap()
        .publish(&source(91),26).unwrap().digest;
    assert_ne!(old,new);
    let second=structured_bundle::prepare_from_cached(&db,ROOT,&new).unwrap();
    let new_bundle=structured_bundle::publish_approved(&core,&vault,&second,true)
        .unwrap().relative_path;
    let new_bytes=fs::read(vault.join(&new_bundle)).unwrap();
    let mut session=BasesSession::new(Arc::new(||{}));
    session.open(vault.clone(),ROOT.into()).unwrap();finish(&mut session);
    assert_eq!(session.view().stage,Stage::Ready);
    assert_eq!(session.view().choices.len(),2);
    session.select(old.clone(),base.clone()).unwrap();finish(&mut session);
    assert_eq!(session.view().local_revision,1);
    session.preview_new_snapshot(&new).unwrap();finish(&mut session);
    let preview=session.view().rebase_preview.unwrap();
    assert_eq!(preview.conflicted_cells,1);
    assert_eq!(preview.cells[0].disposition,
        bases_edits::RebaseDisposition::SourceValueChanged);
    assert!(session.toggle_accept_notion(&row,&property));
    assert!(rebase::manual_choices_complete(&session.view()));
    session.apply_reviewed_snapshot(Arc::clone(&core)).unwrap();finish(&mut session);
    assert_eq!(session.view().stage,Stage::Ready);
    assert_eq!(session.view().digest.as_deref(),Some(new.as_str()));
    let archive=session.view().last_archive.unwrap().path;
    assert!(vault.join(&archive).is_file());
    assert_eq!(fs::read(vault.join(&archive)).unwrap(),local_before);
    assert_eq!(fs::read(vault.join(&old_bundle)).unwrap(),old_bytes);
    assert_eq!(fs::read(vault.join(&new_bundle)).unwrap(),new_bytes);
    drop(session);
    drop(core);

    // Construct a new process-equivalent Core and GUI session, with no old
    // session state or cached archive list carried in memory.
    let reopened=Core::open(&app,Arc::new(|_:aquilum_core::CoreEvent|{}));
    reopened.ui_state.resolve_workspace(vault.to_str().unwrap(),2).unwrap();
    let mut session=BasesSession::new(Arc::new(||{}));
    session.open(vault.clone(),ROOT.into()).unwrap();finish(&mut session);
    session.select(new.clone(),base.clone()).unwrap();finish(&mut session);
    assert!(session.view().last_archive.is_none());
    assert!(session.view().archives.is_empty());
    session.discover_archives().unwrap();finish(&mut session);
    let found=session.view().archives;
    assert_eq!(found.len(),1);
    assert_eq!(found[0].path,archive);
    assert_eq!(found[0].cells.len(),1);
    assert_eq!(found[0].cells[0].row_id,row);
    assert_eq!(found[0].cells[0].property_id,property);
    assert_eq!(found[0].cells[0].value_preview,"721");
    session.restore_listed_archive(Arc::clone(&reopened),
        &archive,&row,&property).unwrap();finish(&mut session);
    assert_eq!(session.view().stage,Stage::Ready);
    let state=bases_edits::open(&vault,ROOT,&new).unwrap().unwrap();
    assert_eq!(state.state.local_value(&row,&property),
        Some(&LocalValue::Number(721.0)));
    assert_eq!(fs::read(vault.join(&old_bundle)).unwrap(),old_bytes);
    assert_eq!(fs::read(vault.join(&new_bundle)).unwrap(),new_bytes);
    assert!(vault.join(&archive).is_file());
    session.discover_archives().unwrap();finish(&mut session);
    assert!(session.view().archives.is_empty(),
        "a recovered value must not be offered for restore twice");
}
