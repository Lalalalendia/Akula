//! Explicitly approved, completely conflict-free C5b4 source rebase.
//! Rechecks both verified bundles and the exact overlay hash at publish time.
//! The original source JSON is never changed; File Gate CAS is the only write.
use super::{
    read, relative, registered, source_model, validate, MAX_FILE,
};
use super::rebase_preview::{preview_rebase, RebaseDisposition};
use std::collections::BTreeSet;
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
    /// Number of source-value conflicts resolved by explicit Keep Local choice.
    pub approved_local_wins: usize,
}

/// No partial migration or silent conflict resolution. The expected hash
/// MUST be the one shown in the user's preview and must still match on disk.
/// This function is not reachable from the background refresh coordinator.
pub fn apply_rebase_approved(
    core: &Core, workspace: &Path, source: &str,
    old_digest: &str, new_digest: &str,
    expected_local_hash: &str, approved: bool,
) -> Result<RebaseReceipt, String> {
    apply_rebase_resolved_approved(core,workspace,source,old_digest,new_digest,
        expected_local_hash,&[],approved)
}

/// A narrowly scoped manual conflict decision: preserve the explicitly
/// selected local overrides when only their Notion source VALUE changed.
/// Structural conflicts, source property definition edits and missing rows
/// always block publication. No decision can remove a local override.
pub fn apply_rebase_resolved_approved(
    core: &Core, workspace: &Path, source: &str,
    old_digest: &str, new_digest: &str,
    expected_local_hash: &str,
    keep_local: &[(String,String)], approved: bool,
) -> Result<RebaseReceipt, String> {
    if !approved {
        return Err("Akula Bases rebase needs explicit user confirmation".into());
    }
    let root=registered(core,workspace)?;
    let preview=preview_rebase(&root,source,old_digest,new_digest)?;
    // Exact equality is intentional. The caller must explicitly select EVERY
    // changed source-value cell, never only a convenient subset, and must not
    // authorize extra or duplicate keys. All other conflict kinds are hard holds.
    let requested=keep_local.iter().cloned().collect::<BTreeSet<_>>();
    let affected=preview.cells.iter()
        .filter(|cell|cell.disposition==RebaseDisposition::SourceValueChanged)
        .map(|cell|(cell.row_id.clone(),cell.property_id.clone()))
        .collect::<BTreeSet<_>>();
    if keep_local.len()>256 || requested.len()!=keep_local.len() || requested!=affected {
        return Err("explicit Keep Local decisions must exactly match all changed source cells".into());
    }
    if preview.conflicted_columns>0 || preview.cells.iter().any(|cell|
        !matches!(cell.disposition,
            RebaseDisposition::Eligible|RebaseDisposition::SourceValueChanged)) {
        return Err("structural or schema rebase conflicts require separate manual review".into());
    }
    if !affected.is_empty() && preview.conflicted_cells!=affected.len() {
        return Err("unresolved source conflicts: rebase denied".into());
    }
    if affected.is_empty() && !preview.can_offer_approval() {
        return Err("no eligible local changes or incompatible schema".into());
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
        source_unchanged:true,approved_local_wins:affected.len(),
    })
}
