//! Explicitly approved, completely conflict-free C5b4 source rebase.
//! Rechecks both verified bundles and the exact overlay hash at publish time.
//! The original source JSON is never changed; File Gate CAS is the only write.
use super::{
    read, relative, registered, source_model, validate, MAX_FILE,
};
use super::rebase_preview::preview_rebase;
use crate::{app_core::Core, files::gate, history::Source};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct RebaseReceipt {
    pub old_digest: String,
    pub new_digest: String,
    pub revision: u64,
    pub disk_hash: String,
    pub overrides_preserved: usize,
    pub local_columns_preserved: usize,
    pub source_unchanged: bool,
}

/// No partial migration or silent conflict resolution. The expected hash
/// MUST be the one shown in the user's preview and must still match on disk.
/// This function is not reachable from the background refresh coordinator.
pub fn apply_rebase_approved(
    core: &Core, workspace: &Path, source: &str,
    old_digest: &str, new_digest: &str,
    expected_local_hash: &str, approved: bool,
) -> Result<RebaseReceipt, String> {
    if !approved {
        return Err("Akula Bases rebase needs explicit user confirmation".into());
    }
    let root=registered(core,workspace)?;
    let preview=preview_rebase(&root,source,old_digest,new_digest)?;
    if !preview.can_offer_approval() {
        return Err("source or local schema conflicts: manual review required".into());
    }
    if preview.expected_local_hash != expected_local_hash {
        return Err("local edits changed since preview: reload before rebase".into());
    }
    let old=source_model(&root,source,old_digest)?;
    let new=source_model(&root,source,new_digest)?;
    let path=root.join(relative(source));
    let current=read(&path,&old)?
        .ok_or("pinned local edits disappeared before approved rebase")?;
    if current.expected_disk_hash!=expected_local_hash
        || current.state.revision!=preview.local_revision {
        return Err("stale local revision: rebase denied".into());
    }
    let mut revised=current.state;
    revised.baseline_digest=new_digest.into();
    revised.revision=revised.revision.checked_add(1)
        .ok_or("Akula Bases revision exhausted")?;
    validate(&new,&revised)?;
    let json=serde_json::to_string(&revised)
        .map_err(|_|"cannot encode rebased Akula edits")?;
    if json.len()>MAX_FILE {
        return Err("rebased edits exceed size limit".into());
    }
    let saved=gate::write(core,&path,&json,Some(expected_local_hash),
        Source::Me,Some("Akula Bases approved source rebase"))
        .map_err(|_|"File Gate rejected rebase CAS; inspect before retry")?;
    let readback=read(&path,&new)?
        .ok_or("rebased local edits disappeared after File Gate")?;
    if readback.expected_disk_hash!=saved.hash
        || readback.state.revision!=revised.revision
        || readback.state.baseline_digest!=new_digest {
        return Err("rebased overlay changed during readback; review required".into());
    }
    Ok(RebaseReceipt {
        old_digest:old_digest.into(),new_digest:new_digest.into(),
        revision:revised.revision,disk_hash:saved.hash,
        overrides_preserved:revised.overrides.len(),
        local_columns_preserved:revised.local_columns.len(),
        source_unchanged:true,
    })
}
