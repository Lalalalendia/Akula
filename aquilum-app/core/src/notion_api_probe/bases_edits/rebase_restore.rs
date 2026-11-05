//! Reapply a specific archived local override to the current verified snapshot.
//! This is never automatic and never replaces the whole overlay from a backup.
use super::{
    apply_approved, read, registered, relative, source_model,
    validate, validate_cell, Change, EditReceipt, Edits, MAX_FILE,
};
use super::rebase_archive::archive_relative;
use crate::app_core::Core;
use std::{fs, path::Path};

/// Restore one local value, not the old baseline or unrelated columns.
/// The archive is bound to both digests and its *exact* prior disk hash.
pub fn restore_archived_cell_approved(
    core:&Core, workspace:&Path, source:&str,
    old_digest:&str, current_digest:&str,
    archived_hash:&str, current_hash:&str,
    archive_name:&str, row_id:&str, property_id:&str,
    approved:bool,
)->Result<EditReceipt,String>{
    if !approved {return Err("archive restore requires explicit confirmation".into());}
    let root=registered(core,workspace)?;
    let expected_name=archive_relative(
        source,old_digest,current_digest,archived_hash);
    if archive_name!=expected_name {
        return Err("archive identity does not match expected source revision".into());
    }
    let old=source_model(&root,source,old_digest)?;
    let new=source_model(&root,source,current_digest)?;
    let archive=root.join(&expected_name);
    let meta=fs::symlink_metadata(&archive)
        .map_err(|_|"Akula Bases prior archive is missing")?;
    #[cfg(windows)]
    let reparse={
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x0000_0400 != 0
    };
    #[cfg(not(windows))]
    let reparse=false;
    if !meta.is_file() || meta.file_type().is_symlink()
        || reparse || meta.len()>MAX_FILE as u64 {
        return Err("unsafe or oversized prior Akula Bases archive".into());
    }
    let bytes=fs::read(&archive)
        .map_err(|_|"cannot read Akula Bases prior archive")?;
    if bytes.len()>MAX_FILE
        || blake3::hash(&bytes).to_hex().to_string()!=archived_hash {
        return Err("prior archive changed: restore refused".into());
    }
    let previous:Edits=serde_json::from_slice(&bytes)
        .map_err(|_|"prior archive is malformed JSON")?;
    validate(&old,&previous)?;
    let previous_value=previous.local_value(row_id,property_id)
        .ok_or("this archived revision contains no such local value")?
        .clone();
    let active=read(&root.join(relative(source)),&new)?
        .ok_or("current local overlay is missing")?;
    if active.expected_disk_hash!=current_hash {
        return Err("current local revision changed: reload before restoring".into());
    }
    if active.state.local_value(row_id,property_id).is_some() {
        return Err("current cell has a newer local override: do not clobber it".into());
    }
    validate_cell(&new,&active.state,row_id,property_id,&previous_value)?;
    apply_approved(core,&root,source,current_digest,Some(current_hash),
        &[Change::Set{row_id:row_id.into(),property_id:property_id.into(),
            value:previous_value}],true)
}
