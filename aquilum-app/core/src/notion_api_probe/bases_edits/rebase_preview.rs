//! C5b4: evidence-only, read-only three-way Akula Bases rebase preview.
//! No File Gate operation is reachable here. Nothing is silently replayed.
//! Old source, new source and the pinned local overlay are all verified first.
use super::{read, relative, source_model, validate_cell, validate_columns_only, Edits, LocalValue};
use crate::notion_api_probe::bases_model::{BaseRow, BasesModel};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseDisposition {
    Eligible,
    RowRemoved,
    RowMoved,
    PropertyRemoved,
    PropertyTypeChanged,
    PropertyDefinitionChanged,
    SourceValueChanged,
    LocalColumnConflict,
    InvalidNewTarget,
}
#[derive(Clone, Debug, Serialize)]
pub struct RebaseCell {
    pub row_id: String,
    pub property_id: String,
    pub disposition: RebaseDisposition,
    /// Complete, scalar/plain-text value previews; none when unsupported or too long.
    pub previous_source: Option<String>,
    pub updated_source: Option<String>,
    pub local_override: Option<String>,
    pub manual_review_supported: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct RebaseColumn {
    pub property_id: String,
    pub base_id: String,
    pub name: String,
    pub compatible: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct RebasePreview {
    pub source_root: String,
    pub old_digest: String,
    pub new_digest: String,
    /// Pin to the exact on-disk local edits. An eventual approved rebase MUST
    /// verify it again immediately before its own File Gate CAS operation.
    pub expected_local_hash: String,
    pub local_revision: u64,
    pub ready_cells: usize,
    pub conflicted_cells: usize,
    pub conflicted_columns: usize,
    pub cells: Vec<RebaseCell>,
    pub columns: Vec<RebaseColumn>,
    /// Previewing never changes the source JSON or the local override file.
    pub writes_performed: bool,
}
impl RebasePreview {
    /// Approval may be offered in a *separate* C5b4 step only when every
    /// element has an unambiguous identity and unchanged source dependency.
    pub fn can_offer_approval(&self) -> bool {
        self.conflicted_cells == 0 && self.conflicted_columns == 0
            && (self.ready_cells > 0 || !self.columns.is_empty())
    }
}
fn source_cell<'a>(
    model: &'a BasesModel, row: &'a BaseRow, property: &str,
) -> Option<(&'a str, &'a serde_json::Value, &'a serde_json::Value)> {
    let base = model.bases.iter().find(|b|
        Some(b.akula_id.as_str()) == row.akula_base_id.as_deref())?;
    let col = base.columns.iter().find(|c|
        c.akula_property_id.as_deref() == Some(property))?;
    let cell = row.cells.iter().find(|c|
        c.akula_property_id.as_deref() == Some(property))?;
    Some((&col.source_type, &col.source_definition, &cell.source_value))
}

const MAX_MANUAL_VALUE_CHARS: usize = 240;
fn bounded_readable(text:String)->Option<String>{
    if text.chars().count()>MAX_MANUAL_VALUE_CHARS
        || text.chars().any(|c|c.is_control() && c!='\n' && c!='\t') {
        return None;
    }
    Some(text)
}
fn readable_source(value:&serde_json::Value)->Option<String>{
    let kind=value.get("type")?.as_str()?;
    let text=match kind {
        "number" => value.get("number")?.as_number()
            .map(ToString::to_string)
            .or_else(||value.get("number")?.is_null().then(||"(пусто)".into()))?,
        "checkbox" => value.get("checkbox")?.as_bool()?.to_string(),
        "title"|"rich_text" => value.get(kind)?.as_array()?.iter()
            .map(|part|part.get("plain_text")?.as_str())
            .collect::<Option<Vec<_>>>()?.join(""),
        "url"|"email"|"phone_number" => value.get(kind)?.as_str()?.to_owned(),
        _=>return None,
    };
    bounded_readable(text)
}
fn readable_local(value:&LocalValue)->Option<String>{
    let text=match value {
        LocalValue::Text(s)=>s.clone(),
        LocalValue::Number(n) if n.is_finite()=>n.to_string(),
        LocalValue::Checkbox(b)=>b.to_string(),
        _=>return None,
    };
    bounded_readable(text)
}

/// All three inputs must belong to the SAME Notion root and a verified,
/// independent Vault. The two source digests must be different; the old
/// local overlay must still be pinned to the old digest.
///
/// This operation is read-only and does not authorize any file modification.
pub fn preview_rebase(
    workspace: &Path, source: &str, old_digest: &str, new_digest: &str,
) -> Result<RebasePreview, String> {
    if old_digest == new_digest {
        return Err("rebase preview requires two distinct source snapshots".into());
    }
    let root = std::fs::canonicalize(workspace)
        .map_err(|_| "Akula Vault unavailable")?;
    let old = source_model(&root, source, old_digest)?;
    let new = source_model(&root, source, new_digest)?;
    if old.source_root != new.source_root || old.source_root != source {
        return Err("Notion root identity changed: cross-root rebase forbidden".into());
    }
    let local = read(&root.join(relative(source)), &old)?
        .ok_or("no local Akula Bases edits exist for this old snapshot")?;
    let mut revised: Edits = local.state.clone();
    revised.baseline_digest = new_digest.into();
    // A collision affects the entire schema transaction: no automatic rename
    // of the user's locally authored columns is ever a safe default.
    let schema_compatible = validate_columns_only(&new, &revised).is_ok();
    let mut columns = Vec::with_capacity(revised.local_columns.len());
    for col in &revised.local_columns {
        let compatible = new.bases.iter().find(|b|b.akula_id == col.base_id
            && b.schema_available).is_some_and(|base|
            !base.columns.iter().any(|s|
                s.name.to_lowercase() == col.name.to_lowercase()
                || s.akula_property_id.as_deref() == Some(col.property_id.as_str())));
        columns.push(RebaseColumn {
            property_id: col.property_id.clone(),
            base_id: col.base_id.clone(), name: col.name.clone(),
            compatible: compatible && schema_compatible,
        });
    }
    let old_rows = old.rows.iter().map(|r|(r.akula_id.as_str(),r))
        .collect::<BTreeMap<_,_>>();
    let new_rows = new.rows.iter().map(|r|(r.akula_id.as_str(),r))
        .collect::<BTreeMap<_,_>>();
    let local_ids = revised.local_columns.iter()
        .map(|c|c.property_id.as_str()).collect::<std::collections::BTreeSet<_>>();
    let mut cells = Vec::with_capacity(revised.overrides.len());
    let mut ready_cells = 0;
    let mut conflicted_cells = 0;
    for override_cell in &revised.overrides {
        let old_row = old_rows.get(override_cell.row_id.as_str())
            .ok_or("validated local row vanished from old baseline")?;
        let target = new_rows.get(override_cell.row_id.as_str()).copied();
        let disposition = match target {
            None => RebaseDisposition::RowRemoved,
            Some(row) if row.akula_base_id != old_row.akula_base_id
                || row.notion_page_id != old_row.notion_page_id =>
                    RebaseDisposition::RowMoved,
            Some(_) if !schema_compatible => RebaseDisposition::LocalColumnConflict,
            Some(row) if local_ids.contains(override_cell.property_id.as_str()) => {
                if validate_cell(&new, &revised, &override_cell.row_id,
                    &override_cell.property_id, &override_cell.value).is_ok() {
                    RebaseDisposition::Eligible
                } else {RebaseDisposition::InvalidNewTarget}
            }
            Some(row) => {
                match (source_cell(&old, old_row, &override_cell.property_id),
                    source_cell(&new, row, &override_cell.property_id)) {
                    (Some(_), None) => RebaseDisposition::PropertyRemoved,
                    (Some((old_type,_,_)), Some((new_type,_,_)))
                        if old_type != new_type =>
                            RebaseDisposition::PropertyTypeChanged,
                    (Some((_,old_definition,_)), Some((_,new_definition,_)))
                        if old_definition != new_definition =>
                            RebaseDisposition::PropertyDefinitionChanged,
                    (Some((_,_,old_value)), Some((_,_,new_value)))
                        if old_value != new_value =>
                            RebaseDisposition::SourceValueChanged,
                    (Some(_), Some(_)) if validate_cell(&new, &revised,
                        &override_cell.row_id, &override_cell.property_id,
                        &override_cell.value).is_ok() =>
                            RebaseDisposition::Eligible,
                    _ => RebaseDisposition::InvalidNewTarget,
                }
            }
        };
        if disposition == RebaseDisposition::Eligible {ready_cells+=1;}
        else {conflicted_cells+=1;}
        let (previous_source,updated_source,local_override,manual_review_supported)=
            if disposition==RebaseDisposition::SourceValueChanged {
                let prior=source_cell(&old,old_row,&override_cell.property_id)
                    .and_then(|(_,_,value)|readable_source(value));
                let incoming=target.and_then(|row|
                    source_cell(&new,row,&override_cell.property_id)
                        .and_then(|(_,_,value)|readable_source(value)));
                let local=readable_local(&override_cell.value);
                // A raw Notion edit that leaves the complete visible value
                // unchanged (e.g. rich-text styling only) needs a richer
                // review UI; never pretend that this scalar preview explains it.
                let supported=prior.is_some() && incoming.is_some() && local.is_some()
                    && prior!=incoming;
                (prior,incoming,local,supported)
            } else {(None,None,None,false)};
        cells.push(RebaseCell {
            row_id:override_cell.row_id.clone(),
            property_id:override_cell.property_id.clone(),disposition,
            previous_source,updated_source,local_override,manual_review_supported,
        });
    }
    let conflicted_columns=columns.iter().filter(|c|!c.compatible).count();
    Ok(RebasePreview {
        source_root:source.into(),old_digest:old_digest.into(),
        new_digest:new_digest.into(),
        expected_local_hash:local.expected_disk_hash,
        local_revision:local.state.revision,
        ready_cells,conflicted_cells,conflicted_columns,cells,columns,
        writes_performed:false,
    })
}
