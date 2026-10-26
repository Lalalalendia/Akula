//! Bounded, read-only discovery of prior Akula Bases backups after restart.
//! No API token, network call, or File Gate mutation is reachable here.
use super::{
    read, relative, source_model, validate, validate_cell, validate_folder,
    Edits, LocalValue, DIR, MAX_FILE,
};
use super::rebase_archive::archive_relative;
use serde::Serialize;
use std::{fs, path::Path};

const MAX_DIR_ENTRIES: usize = 1024;
const MAX_ARCHIVES: usize = 16;
const MAX_VISIBLE_CELLS: usize = 10;
const MAX_PREVIEW_CHARS: usize = 240;

#[derive(Clone, Debug, Serialize)]
pub struct RestoreCell {
    pub row_id: String,
    pub property_id: String,
    pub value_preview: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct RestoreArchive {
    pub path: String,
    pub old_digest: String,
    pub new_digest: String,
    pub archived_hash: String,
    pub cells: Vec<RestoreCell>,
}
fn visible(value:&LocalValue)->Option<String>{
    let raw=match value {
        LocalValue::Text(v)=>v.clone(),
        LocalValue::Number(v) if v.is_finite()=>v.to_string(),
        LocalValue::Checkbox(v)=>v.to_string(),
        _=>return None,
    };
    if raw.chars().count()>MAX_PREVIEW_CHARS ||
        raw.chars().any(|c|c.is_control() && c!='\n' && c!='\t') {
        None
    }else{Some(raw)}
}
fn digest_lowercase(value:&str)->bool{
    value.len()==64 && value.bytes().all(|b|
        b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Read-only scan. The currently active sidecar must match the UI hash on
/// both sides of discovery; the restore Core operation revalidates everything
/// and performs its own fresh File Gate CAS.
pub fn discover_restore_archives(
    workspace:&Path, source:&str, current_digest:&str, expected_hash:&str,
    base_id:&str,
)->Result<Vec<RestoreArchive>,String>{
    let root=fs::canonicalize(workspace)
        .map_err(|_|"Akula Vault unavailable")?;
    validate_folder(&root)?;
    let current_model=source_model(&root,source,current_digest)?;
    if !current_model.bases.iter().any(|b|
        b.akula_id==base_id && b.schema_available) {
        return Err("unknown or unavailable target Akula Base".into());
    }
    let active_path=root.join(relative(source));
    let before=read(&active_path,&current_model)?
        .ok_or("no active local overlay to recover into")?;
    if before.expected_disk_hash!=expected_hash {
        return Err("local edits changed before archive discovery".into());
    }
    let mut archives=Vec::new();
    let folder=root.join(DIR);
    let mut count=0usize;
    let mut matching=0usize;
    for entry in fs::read_dir(&folder)
        .map_err(|_|"cannot enumerate Akula Bases archive directory")? {
        let entry=entry.map_err(|_|"cannot read Akula Bases directory entry")?;
        count+=1;
        if count>MAX_DIR_ENTRIES {
            return Err("too many files to safely enumerate local archives".into());
        }
        let filename=entry.file_name();
        let Some(name)=filename.to_str() else {continue};
        let Some(archive_id)=name.strip_prefix(&format!("{source}-"))
            .and_then(|s|s.strip_suffix(".akula-bases-backup.json"))
            else{continue};
        if !digest_lowercase(archive_id) {
            return Err("invalid Akula Bases archive filename".into());
        }
        matching+=1;
        if matching>MAX_ARCHIVES {
            return Err("archive count exceeds bounded QA scan; review required".into());
        }
        let path=entry.path();
        let meta=fs::symlink_metadata(&path)
            .map_err(|_|"cannot inspect Akula Bases archived file")?;
        #[cfg(windows)]
        let reparse={
            use std::os::windows::fs::MetadataExt;
            meta.file_attributes() & 0x0000_0400 != 0
        };
        #[cfg(not(windows))]
        let reparse=false;
        if !meta.is_file() || meta.file_type().is_symlink()
            || reparse || meta.len()>MAX_FILE as u64 {
            return Err("unsafe archived Akula Bases file".into());
        }
        let bytes=fs::read(&path)
            .map_err(|_|"cannot read prior Akula Bases archive")?;
        if bytes.len()>MAX_FILE {return Err("oversized prior archive".into());}
        let archived_hash=blake3::hash(&bytes).to_hex().to_string();
        let previous:Edits=serde_json::from_slice(&bytes)
            .map_err(|_|"malformed prior Akula Bases archive")?;
        if previous.source_root!=source {
            return Err("invalid archived source identity".into());
        }
        if previous.baseline_digest==current_digest {
            // A backup made before switching AWAY from this snapshot cannot
            // restore a value into the same unmodified source snapshot.
            continue;
        }
        let expected_relative=archive_relative(
            source,&previous.baseline_digest,current_digest,&archived_hash);
        if expected_relative!=format!("{DIR}/{name}") {
            // Older backup for a different current digest is not a candidate.
            // It is immutable and may be relevant after another snapshot switch.
            continue;
        }
        let old_model=source_model(&root,source,&previous.baseline_digest)?;
        validate(&old_model,&previous)?;
        let mut cells=Vec::new();
        for item in &previous.overrides {
            if before.state.local_value(&item.row_id,&item.property_id).is_some() {
                continue;
            }
            if !current_model.rows.iter().any(|row|
                row.akula_id==item.row_id
                && row.akula_base_id.as_deref()==Some(base_id)) {
                continue;
            }
            if validate_cell(&current_model,&before.state,
                &item.row_id,&item.property_id,&item.value).is_err() {continue;}
            let Some(value_preview)=visible(&item.value) else{continue};
            if cells.len()>=MAX_VISIBLE_CELLS {
                return Err("too many archived cells for bounded manual review".into());
            }
            cells.push(RestoreCell {
                row_id:item.row_id.clone(),
                property_id:item.property_id.clone(),value_preview,
            });
        }
        if !cells.is_empty() {
            archives.push(RestoreArchive {
                path:expected_relative,
                old_digest:previous.baseline_digest,
                new_digest:current_digest.into(),
                archived_hash,cells,
            });
        }
    }
    let after=read(&active_path,&current_model)?
        .ok_or("active overlay disappeared during archive discovery")?;
    if after.expected_disk_hash!=expected_hash {
        return Err("local overlay changed during archive discovery".into());
    }
    archives.sort_by(|a,b|a.path.cmp(&b.path));
    Ok(archives)
}
