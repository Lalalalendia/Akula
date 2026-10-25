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
