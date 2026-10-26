use super::*;
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

#[test]
fn c5b4_cold_reopen_preserves_backup_and_restores_one_cell_by_file_gate_cas() {
    let f=Fixture::new();
    let row=f.row(A);
    let property=f.prop(DS1,"score");
    let other_row=f.row(C);
    let other_property=f.prop(DS1,"done");
    let base=f.model().bases.iter().find(|b|b.notion_id==DS1)
        .unwrap().akula_id.clone();
    let first=apply_approved(&f.core,&f.vault,A,&f.digest,None,&[
        Change::Set{row_id:row.clone(),property_id:property.clone(),
            value:LocalValue::Number(721.0)},
        Change::Set{row_id:other_row.clone(),property_id:other_property.clone(),
            value:LocalValue::Checkbox(true)},
    ],true).unwrap();
    let old_bundle=fs::read(f.vault.join(&f.bundle)).unwrap();
    let old_overlay=fs::read(f.edited_path()).unwrap();
    let next=published_new_source(&f,|source|{
        source.objects.iter_mut().find(|o|o.kind=="page"&&o.id==A)
            .unwrap().json["properties"]["Score"]["number"]=json!(94);
    });
    let newer_bundle=format!("Notion-import/{A}-{next}.notion.json");
    let incoming_before=fs::read(f.vault.join(&newer_bundle)).unwrap();
    let apply=apply_rebase_decisions_approved(&f.core,&f.vault,A,&f.digest,&next,
        &first.disk_hash,
        &[(row.clone(),property.clone(),RebaseDecision::AcceptNotion)],
        true).unwrap();
    assert_eq!(apply.overrides_preserved,1);
    let archived=apply.pre_rebase_archive.unwrap();
    assert_eq!(fs::read(f.vault.join(&archived)).unwrap(),old_overlay);

    // A fresh Core with the same persisted app state simulates process
    // reopening. The old Core is explicitly dropped first.
    let Fixture{_temp,core,app,vault,digest,bundle}=f;
    drop(core);
    let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(),1).unwrap();
    let reopened=Fixture{_temp,core,app,vault,digest,bundle};
    let choices=discover_restore_archives(&reopened.vault,A,&next,
        &apply.disk_hash,&base).unwrap();
    assert_eq!(choices.len(),1);
    assert_eq!(choices[0].path,archived);
    assert_eq!(choices[0].cells.len(),1);
    assert_eq!(choices[0].cells[0].row_id,row);
    assert_eq!(choices[0].cells[0].property_id,property);
    assert_eq!(choices[0].cells[0].value_preview,"721");

    let restored=restore_archived_cell_approved(
        &reopened.core,&reopened.vault,A,&reopened.digest,&next,
        &first.disk_hash,&apply.disk_hash,&archived,
        &row,&property,true,
    ).unwrap();
    assert_eq!(restored.revision,apply.revision+1);
    let current=open(&reopened.vault,A,&next).unwrap().unwrap();
    assert_eq!(current.state.local_value(&row,&property),
        Some(&LocalValue::Number(721.0)));
    assert_eq!(current.state.local_value(&other_row,&other_property),
        Some(&LocalValue::Checkbox(true)));
    assert_eq!(fs::read(reopened.vault.join(&archived)).unwrap(),old_overlay);
    assert_eq!(fs::read(reopened.vault.join(&reopened.bundle)).unwrap(),old_bundle);
    assert_eq!(fs::read(reopened.vault.join(&newer_bundle)).unwrap(),incoming_before);
    assert!(discover_restore_archives(&reopened.vault,A,&next,
        &restored.disk_hash,&base).unwrap().is_empty());
}
