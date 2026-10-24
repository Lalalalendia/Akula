//! API-root-scoped typed base/query model for future independent Akula Bases.
//!
//! This model is produced from Akula's fully verified local API snapshot; it
//! requires no token and has no network or File Gate authority. The original
//! typed JSON bundle remains the canonical evidence of source fidelity.
//!
//! There is NO native editable Bases engine yet: expressions, rollups, views
//! and source relations remain inert; typed values are snapshots only.
use super::{
    identity_registry::{node_id, property_id},
    snapshot::SnapshotStore, RawObject, SourcePreview, API_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use uuid::Uuid;

pub const MODEL_FORMAT: &str = "akula_notion_bases_model_v1";
const MAX_SOURCE_ID: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseColumn {
    pub akula_property_id: Option<String>,
    pub notion_property_id: Option<String>,
    pub name: String,
    pub source_type: String,
    /// Full source schema definition, including select options, formula
    /// expressions, rollup relationship config and unknown future fields.
    pub source_definition: Value,
    pub native_edit_supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseSchema {
    pub akula_id: String,
    pub notion_id: String,
    pub notion_kind: String,
    pub display_name: Option<String>,
    pub columns: Vec<BaseColumn>,
    pub schema_available: bool,
    pub native_edit_supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Binding {
    Exact,
    NoPropertyId,
    NoSchema,
    UnknownProperty,
    TypeMismatch,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseCell {
    pub notion_property_id: Option<String>,
    pub akula_property_id: Option<String>,
    pub source_type: String,
    pub binding: Binding,
    /// Exact Notion page-property response, not an interpreted formula value.
    pub source_value: Value,
    pub native_edit_supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseRow {
    pub akula_id: String,
    pub notion_page_id: String,
    pub notion_parent_id: String,
    pub parent_kind: String,
    pub akula_base_id: Option<String>,
    pub source_revision: Option<String>,
    pub cells: Vec<BaseCell>,
    pub native_edit_supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseView {
    pub akula_id: String,
    pub notion_view_id: String,
    pub notion_owner_id: Option<String>,
    pub akula_base_id: Option<String>,
    pub view_type: String,
    /// Retain filter/sort/group/visible-column configuration without
    /// claiming the local renderer supports it.
    pub source_configuration: Value,
    pub native_render_supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BasesModel {
    pub format: String,
    pub source_provider: String,
    pub source_scope_kind: String,
    pub source_root: String,
    pub source_digest: String,
    pub workspace_identity_verified: bool,
    pub cross_root_merge_supported: bool,
    pub native_edit_supported: bool,
    pub views_rendered_natively: bool,
    pub bases: Vec<BaseSchema>,
    pub rows: Vec<BaseRow>,
    pub views: Vec<BaseView>,
    pub unbound_rows: usize,
    pub unbound_views: usize,
    pub fields_needing_review: usize,
    pub source_relation_edges: usize,
}
impl BasesModel {
    pub fn rows_for_base<'a>(&'a self, akula_id: &'a str) -> impl Iterator<Item = &'a BaseRow> + 'a {
        self.rows.iter().filter(move |r| r.akula_base_id.as_deref() == Some(akula_id))
    }
    pub fn views_for_base<'a>(&'a self, akula_id: &'a str) -> impl Iterator<Item = &'a BaseView> + 'a {
        self.views.iter().filter(move |v| v.akula_base_id.as_deref() == Some(akula_id))
    }
    pub fn needs_review(&self) -> bool {
        self.unbound_rows > 0 || self.unbound_views > 0 ||
            self.fields_needing_review > 0
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_SOURCE_ID && !id.chars().any(char::is_control)
}
fn property_type(p: &Value) -> &str {
    p.get("type").and_then(Value::as_str).unwrap_or("unknown")
}
fn source_parent(json: &Value) -> Option<(&'static str, &str)> {
    let parent = json.get("parent")?;
    match parent.get("type")?.as_str()? {
        "data_source_id" => parent.get("data_source_id")?.as_str().map(|s| ("data_source",s)),
        "database_id" => parent.get("database_id")?.as_str().map(|s| ("database",s)),
        _ => None,
    }
}
fn display_name(json: &Value) -> Option<String> {
    if let Some(name) = json.get("name").and_then(Value::as_str) {
        return Some(name.to_owned());
    }
    let titles = json.get("title")?.as_array()?;
    Some(titles.iter().filter_map(|v|v.get("plain_text").and_then(Value::as_str))
        .collect::<String>())
}
fn validate_raw_object(object: &RawObject) -> Result<(), String> {
    if object.kind.is_empty() || object.kind.len()>64 || !valid_id(&object.id) {
        return Err("invalid raw Notion object kind/identity".into());
    }
    Ok(())
}
fn columns(root:&str, owner_kind:&str, owner:&str, json:&Value)
    -> Result<(Vec<BaseColumn>, bool), String>
{
    let Some(props) = json.get("properties").and_then(Value::as_object) else {
        return Ok((Vec::new(),false));
    };
    let mut columns=Vec::new();
    let mut seen=BTreeSet::new();
    for (name, definition) in props {
        let id=definition.get("id").and_then(Value::as_str);
        if id.is_some_and(|id|!valid_id(id)) {
            return Err("invalid Notion schema property ID".into());
        }
        if let Some(id) = id {
            if !seen.insert(id.to_owned()) {
                return Err("duplicate Notion schema property ID".into());
            }
        }
        columns.push(BaseColumn {
            akula_property_id:id.map(|id|property_id(root,owner_kind,owner,id)),
            notion_property_id:id.map(str::to_owned),name:name.clone(),
            source_type:property_type(definition).to_owned(),
            source_definition:definition.clone(),
            native_edit_supported:false,
        });
    }
    columns.sort_by(|a,b|(&a.notion_property_id,&a.name)
        .cmp(&(&b.notion_property_id,&b.name)));
    Ok((columns,true))
}
fn cells(page: &Value, schema: Option<&BaseSchema>) -> Result<Vec<BaseCell>, String> {
    let Some(properties)=page.get("properties").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut result=Vec::new();
    let mut seen=BTreeSet::new();
    for value in properties.values() {
        let id=value.get("id").and_then(Value::as_str);
        if id.is_some_and(|id|!valid_id(id)) {
            return Err("invalid Notion row property ID".into());
        }
        if let Some(id)=id {
            if !seen.insert(id.to_owned()) {
                return Err("duplicate Notion row property ID".into());
            }
        }
        let expected=id.and_then(|id|schema.and_then(|s|s.columns.iter().find(|c|
            c.notion_property_id.as_deref()==Some(id))));
        let binding=if id.is_none(){Binding::NoPropertyId}
            else if schema.is_none(){Binding::NoSchema}
            else if let Some(definition)=expected {
                if definition.source_type==property_type(value){Binding::Exact}
                else{Binding::TypeMismatch}
            } else {Binding::UnknownProperty};
        result.push(BaseCell{
            notion_property_id:id.map(str::to_owned),
            akula_property_id:if binding==Binding::Exact {
                expected.and_then(|p|p.akula_property_id.clone())
            } else {None},
            source_type:property_type(value).to_owned(),
            binding,source_value:value.clone(),native_edit_supported:false,
        });
    }
    result.sort_by(|a,b|(&a.notion_property_id,&a.source_type)
        .cmp(&(&b.notion_property_id,&b.source_type)));
    Ok(result)
}
fn compose(source: &SourcePreview, digest: &str) -> Result<BasesModel,String> {
    if source.api_version != API_VERSION || !source.root_traversal_complete
        || source.workspace_complete || !source.unresolved_target_ids.is_empty()
        || !source.warnings.is_empty()
        || Uuid::parse_str(&source.root).map(|u|u.to_string()).ok().as_deref()
            != Some(source.root.as_str())
        || digest.len()!=64 || !digest.bytes().all(|b|b.is_ascii_hexdigit())
    { return Err("Notion model requires verified, complete authorized root".into()); }
    let mut seen=BTreeSet::new();
    let mut by_source=BTreeMap::new();
    for object in &source.objects {
        if object.kind=="markdown" {continue;}
        validate_raw_object(object)?;
        if !seen.insert((&object.kind,&object.id)) {
            return Err("duplicated Notion entity in bases model".into());
        }
        if object.kind=="data_source" || object.kind=="database" {
            let (fields,available)=columns(&source.root,&object.kind,&object.id,&object.json)?;
            let model=BaseSchema{
                akula_id:node_id(&source.root,&object.kind,&object.id),
                notion_id:object.id.clone(),notion_kind:object.kind.clone(),
                display_name:display_name(&object.json),
                columns:fields,schema_available:available,native_edit_supported:false,
            };
            by_source.insert((object.kind.clone(),object.id.clone()),model);
        }
    }
    let mut bases=by_source.values().cloned().collect::<Vec<_>>();
    let mut rows=Vec::new();
    let mut views=Vec::new();
    let mut unbound_rows=0;
    let mut unbound_views=0;
    let mut fields_needing_review=bases.iter().map(|s| {
        usize::from(!s.schema_available)+s.columns.iter()
            .filter(|p|p.akula_property_id.is_none()).count()
    }).sum::<usize>();
    for object in &source.objects {
        if object.kind=="page" {
            // Standalone pages are regular Markdown documents, not invented
            // database records. A page with a database/data_source parent is a row.
            let Some((kind,owner))=source_parent(&object.json) else {continue};
            if !valid_id(owner) {return Err("invalid Notion row parent identity".into());}
            let owner_key=(kind.to_owned(),owner.to_owned());
            let schema=by_source.get(&owner_key);
            if schema.is_none(){unbound_rows+=1;}
            let values=cells(&object.json,schema)?;
            fields_needing_review+=values.iter()
                .filter(|v|v.binding!=Binding::Exact).count();
            rows.push(BaseRow {
                akula_id:node_id(&source.root,"page",&object.id),
                notion_page_id:object.id.clone(), notion_parent_id:owner.to_owned(),
                parent_kind:kind.to_owned(),
                akula_base_id:schema.map(|s|s.akula_id.clone()),
                source_revision:object.json.get("last_edited_time")
                    .and_then(Value::as_str).map(str::to_owned),
                cells:values,native_edit_supported:false,
            });
        } else if object.kind=="view" {
            let parent=source_parent(&object.json);
            if parent.is_some_and(|(_,id)|!valid_id(id)){
                return Err("invalid Notion view parent identity".into());
            }
            let base=parent.and_then(|(kind,id)|by_source.get(&(kind.to_owned(),id.to_owned())));
            if base.is_none(){unbound_views+=1;}
            views.push(BaseView {
                akula_id:node_id(&source.root,"view",&object.id),
                notion_view_id:object.id.clone(),
                notion_owner_id:parent.map(|(_,id)|id.to_owned()),
                akula_base_id:base.map(|s|s.akula_id.clone()),
                view_type:object.json.get("type").and_then(Value::as_str)
                    .unwrap_or("unknown").to_owned(),
                source_configuration:object.json.clone(),native_render_supported:false,
            });
        }
    }
    bases.sort_by(|a,b|(&a.notion_kind,&a.notion_id).cmp(&(&b.notion_kind,&b.notion_id)));
    rows.sort_by(|a,b|a.notion_page_id.cmp(&b.notion_page_id));
    views.sort_by(|a,b|a.notion_view_id.cmp(&b.notion_view_id));
    Ok(BasesModel {
        format:MODEL_FORMAT.into(),source_provider:"notion".into(),
        source_scope_kind:"root".into(),source_root:source.root.clone(),
        source_digest:digest.to_ascii_lowercase(),
        workspace_identity_verified:false,cross_root_merge_supported:false,
        native_edit_supported:false,views_rendered_natively:false,
        bases,rows,views,unbound_rows,unbound_views,fields_needing_review,
        source_relation_edges:source.links.len(),
    })
}
/// Metadata/schema/record model derived from a checksum-validated read-only
/// SQLite snapshot. No filesystem writes and no remote read authorization.
pub fn from_verified_cache(path: &Path, root: &str, digest: &str)
    -> Result<BasesModel,String>
{
    let store=SnapshotStore::open_read_only(path)?;
    let cached=store.load(root)?.ok_or("Notion root is absent from verified cache")?;
    if cached.info.digest!=digest {
        return Err("Notion root changed after model review".into());
    }
    compose(&cached.into_preview(),digest)
}
#[cfg(test)]
#[path="bases_model_tests.rs"]
mod tests;
