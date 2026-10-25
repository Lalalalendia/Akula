//! Read-only effective Akula Bases projection: imported typed source + local
//! CAS overrides. This is computed, NEVER a fourth file or mutable Notion API.
use super::{read, relative, source_model, LocalKind, LocalValue};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag="origin",content="data",rename_all="snake_case")]
pub enum EffectiveValue {
    /// The exact source Notion property JSON, including inert formula/rollup.
    Source(Value),
    /// Explicitly user-authored local override, never a Notion server value.
    Local(LocalValue),
    /// Brand-new Akula-only column without a value in this row.
    EmptyLocal,
}
#[derive(Clone, Debug, Serialize)]
pub struct EffectiveColumn {
    pub property_id: String,
    pub name: String,
    pub kind: String,
    pub local_only: bool,
    pub editable_locally: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct EffectiveCell {
    pub property_id: String,
    pub value: EffectiveValue,
}
#[derive(Clone, Debug, Serialize)]
pub struct EffectiveRow {
    pub akula_id: String,
    pub notion_page_id: String,
    pub cells: Vec<EffectiveCell>,
}
#[derive(Clone, Debug, Serialize)]
pub struct EffectiveBase {
    pub base_id: String,
    pub notion_source_id: String,
    pub source_root: String,
    pub source_digest: String,
    pub local_revision: u64,
    pub columns: Vec<EffectiveColumn>,
    pub rows: Vec<EffectiveRow>,
    /// Original view definitions are preserved in the source JSON, not
    /// executed by a guessed local filter/sort/group engine.
    pub saved_source_views: usize,
    pub native_views_supported: bool,
}
fn can_edit_source(kind: &str) -> bool {
    matches!(kind,"title"|"rich_text"|"url"|"email"|"phone_number"
        |"number"|"checkbox"|"select"|"multi_select"|"relation")
}
fn kind_label(kind: &LocalKind) -> &'static str {
    match kind {
        LocalKind::Text=>"rich_text",
        LocalKind::Number=>"number",
        LocalKind::Checkbox=>"checkbox",
    }
}
/// Display exactly one row per imported Notion page. Local-only columns and
/// source overrides are projected in memory; raw source files are untouched.
///
/// Fail closed if a source digest changed, if an overlay has been tampered
/// with, or if its contents are inconsistent with the verified source schema.
pub fn read_effective_base(
    workspace: &Path, source: &str, digest: &str, base_id: &str,
) -> Result<EffectiveBase, String> {
    let root=std::fs::canonicalize(workspace)
        .map_err(|_|"Akula Vault unavailable")?;
    let model=source_model(&root,source,digest)?;
    let schema=model.bases.iter().find(|b|b.akula_id==base_id && b.schema_available)
        .ok_or("selected Akula base does not exist in this source root")?;
    let current=read(&root.join(relative(source)),&model)?;
    let state=current.as_ref().map(|s|&s.state);
    let local_columns=state.into_iter()
        .flat_map(|s|s.local_columns.iter().filter(|col|col.base_id==base_id))
        .collect::<Vec<_>>();
    let mut columns=Vec::new();
    for field in &schema.columns {
        let id=field.akula_property_id.as_ref()
            .ok_or("source property identity cannot be inferred")?;
        columns.push(EffectiveColumn {
            property_id:id.clone(),name:field.name.clone(),
            kind:field.source_type.clone(),local_only:false,
            editable_locally:can_edit_source(&field.source_type),
        });
    }
    for col in &local_columns {
        columns.push(EffectiveColumn {
            property_id:col.property_id.clone(),name:col.name.clone(),
            kind:kind_label(&col.kind).into(),local_only:true,
            editable_locally:true,
        });
    }
    let mut rows=Vec::new();
    for row in model.rows_for_base(base_id) {
        let mut cells=Vec::new();
        for source_cell in &row.cells {
            let Some(id)=source_cell.akula_property_id.as_deref() else {
                return Err("source row contains an unbound property".into());
            };
            let value=state.and_then(|s|s.local_value(&row.akula_id,id))
                .cloned().map(EffectiveValue::Local)
                .unwrap_or_else(||EffectiveValue::Source(source_cell.source_value.clone()));
            cells.push(EffectiveCell{property_id:id.to_owned(),value});
        }
        for col in &local_columns {
            let value=state.and_then(|s|s.local_value(&row.akula_id,&col.property_id))
                .cloned().map(EffectiveValue::Local)
                .unwrap_or(EffectiveValue::EmptyLocal);
            cells.push(EffectiveCell{property_id:col.property_id.clone(),value});
        }
        rows.push(EffectiveRow{
            akula_id:row.akula_id.clone(),
            notion_page_id:row.notion_page_id.clone(),cells,
        });
    }
    Ok(EffectiveBase {
        base_id:base_id.to_owned(),notion_source_id:schema.notion_id.clone(),
        source_root:model.source_root.clone(),source_digest:model.source_digest.clone(),
        local_revision:state.map_or(0,|s|s.revision),columns,rows,
        saved_source_views:model.views_for_base(base_id).count(),
        native_views_supported:false,
    })
}
