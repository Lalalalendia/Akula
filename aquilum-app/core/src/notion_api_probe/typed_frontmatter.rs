//! Conservative Notion property bridge into Akula's existing frontmatter
//! search/Dataview indexer. Original data remains in an independent JSON
//! bundle; do not evaluate a Notion rollup or formula locally.
//!
//! Only bounded, safely representable scalar/list values are projected.
//! Anything unsupported stays in JSON and sets a partial-projection marker.
use serde_json::Value;
use std::collections::BTreeSet;
const MAX_FRONTMATTER_BYTES: usize = 4 * 1024;
const MAX_FIELDS: usize = 64;
const MAX_VALUE_CHARS: usize = 256;
const MAX_LIST_ITEMS: usize = 32;

enum Projection { Scalar(String), List(Vec<String>) }

fn canonical_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).map(|v| v.to_string() == id).unwrap_or(false)
}
fn key(name: &str, property: &Value) -> Option<String> {
    let slug = name.chars().take(36).map(|ch|
        if ch.is_alphanumeric() { ch } else { '_' }
    ).collect::<String>();
    let slug = slug.trim_matches('_');
    if slug.is_empty() { return None; }
    let stable_id = property.get("id").and_then(Value::as_str).unwrap_or(name);
    let digest = blake3::hash(stable_id.as_bytes()).to_hex();
    Some(format!("notion_{slug}_{}", &digest.as_str()[..12]))
}

/// Akula's current frontmatter scanner accepts bounded plain scalars.
/// Refuse characters/leading symbols with YAML structural significance.
fn safe_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_VALUE_CHARS * 4
        && text.chars().count() <= MAX_VALUE_CHARS
        && text.trim() == text
        && !text.chars().any(|ch| ch.is_control()
            || matches!(ch, '"' | '\'' | '\\' | '#' | '[' | ']' | '{' | '}'))
        && !text.contains(": ")
        && !text.chars().next().is_some_and(|ch|
            matches!(ch, '-' | '!' | '&' | '*' | '?' | '|' | '>' | '@' | '%')
                || ch == char::from(96))
        && !text.eq_ignore_ascii_case("true")
        && !text.eq_ignore_ascii_case("false")
        && !text.eq_ignore_ascii_case("null")
        && text.parse::<f64>().is_err()
}
fn text_from(value: &Value) -> Option<Projection> {
    let text = value.as_str()?;
    safe_text(text).then(|| Projection::Scalar(text.to_owned()))
}
fn encode(property: &Value) -> Option<Projection> {
    match property.get("type")?.as_str()? {
        "number" => {
            let number = property.get("number")?.as_f64()?;
            number.is_finite().then(|| Projection::Scalar(number.to_string()))
        }
        "checkbox" => property.get("checkbox")?.as_bool()
            .map(|value| Projection::Scalar(value.to_string())),
        "select" | "status" => {
            let kind = property.get("type")?.as_str()?;
            text_from(property.get(kind)?.get("name")?)
        }
        "date" => {
            let date = property.get("date")?;
            if date.get("end").is_some_and(|end| !end.is_null()) { return None; }
            text_from(date.get("start")?)
        }
        "url" | "email" | "phone_number" => {
            let kind = property.get("type")?.as_str()?;
            text_from(property.get(kind)?)
        }
        "rich_text" => {
            let runs = property.get("rich_text")?.as_array()?;
            if runs.len() > MAX_LIST_ITEMS { return None; }
            let mut joined = String::new();
            for run in runs { joined.push_str(run.get("plain_text")?.as_str()?); }
            safe_text(&joined).then(|| Projection::Scalar(joined))
        }
        "multi_select" => {
            let values = property.get("multi_select")?.as_array()?;
            if values.len() > MAX_LIST_ITEMS { return None; }
            let mut names = Vec::with_capacity(values.len());
            for value in values {
                let name = value.get("name")?.as_str()?;
                if !safe_text(name) { return None; }
                names.push(name.to_owned());
            }
            Some(Projection::List(names))
        }
        // Relation IDs become graph links in Markdown; their original typed
        // metadata, like formulas/rollups/people/files, remains in JSON.
        _ => None,
    }
}
fn rendered(key: &str, value: Projection) -> String {
    match value {
        Projection::Scalar(value) => format!("{key}: {value}\n"),
        Projection::List(items) if items.is_empty() => format!("{key}: []\n"),
        Projection::List(items) => {
            let mut out = format!("{key}:\n");
            for item in items { out.push_str(&format!("  - {item}\n")); }
            out
        }
    }
}
fn parent_id(page: &Value) -> Option<(&'static str, &str)> {
    let parent = page.get("parent")?;
    if let Some(id) = parent.get("data_source_id").and_then(Value::as_str) {
        return canonical_id(id).then_some(("notion_data_source_id", id));
    }
    if let Some(id) = parent.get("database_id").and_then(Value::as_str) {
        return canonical_id(id).then_some(("notion_database_id", id));
    }
    None
}
pub(super) fn for_page(page: &Value, source_id: &str) -> String {
    let mut result = format!("---\nnotion_source_id: {source_id}\n");
    if let Some((name, id)) = parent_id(page) {
        result.push_str(&format!("{name}: {id}\n"));
    }
    let mut partial = false;
    let mut exported = 0_usize;
    let mut used = BTreeSet::new();
    if let Some(props) = page.get("properties").and_then(Value::as_object) {
        for (name, property) in props {
            if property.get("type").and_then(Value::as_str) == Some("title") { continue; }
            if exported >= MAX_FIELDS { partial = true; break; }
            let Some((field, value)) = key(name, property).zip(encode(property))
                else { partial = true; continue; };
            if !used.insert(field.clone()) { partial = true; continue; }
            let line = rendered(&field, value);
            if result.len() + line.len() + 72 > MAX_FRONTMATTER_BYTES {
                partial = true; continue;
            }
            result.push_str(&line);
            exported += 1;
        }
    } else if page.get("properties").is_some() {
        partial = true;
    }
    if partial { result.push_str("notion_query_projection_partial: true\n"); }
    result.push_str("---\n\n");
    result
}

#[cfg(test)]
#[path = "typed_frontmatter_dataview.rs"]
mod dataview_integration;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::fields::{note_fields, Field, FieldKind};
    use serde_json::json;
    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const DS: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    fn field<'a>(fields: &'a [Field], prefix: &str) -> &'a Field {
        fields.iter().find(|f| f.key.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing Notion indexable {prefix}"))
    }
    #[test]
    fn scalar_lists_and_parent_are_queryable_by_existing_akula_indexer() {
        let raw = json!({"parent":{"data_source_id":DS},"properties":{
            "Name":{"type":"title","title":[{"plain_text":"Example"}]},
            "Score":{"id":"S1","type":"number","number":42.5},
            "Done":{"id":"B1","type":"checkbox","checkbox":true},
            "Status":{"id":"P1","type":"status","status":{"name":"In progress"}},
            "Due":{"id":"D1","type":"date","date":{"start":"2026-10-24"}},
            "Labels":{"id":"L1","type":"multi_select","multi_select":[
                {"name":"Research"},{"name":"Needs review"}
            ]},
            "Progress":{"type":"formula","formula":{"number":5}}
        }});
        let md = format!("{}# Example\n", for_page(&raw, A));
        let fields = note_fields(&md);
        assert_eq!(field(&fields, "notion_source_id").text, A);
        assert_eq!(field(&fields, "notion_data_source_id").text, DS);
        assert_eq!(field(&fields, "notion_Score_").kind, FieldKind::Number);
        assert_eq!(field(&fields, "notion_Score_").text, "42.5");
        assert_eq!(field(&fields, "notion_Done_").kind, FieldKind::Bool);
        assert_eq!(field(&fields, "notion_Status_").text, "In progress");
        assert_eq!(field(&fields, "notion_Due_").text, "2026-10-24");
        assert_eq!(field(&fields, "notion_Labels_").kind, FieldKind::List);
        assert_eq!(field(&fields, "notion_Labels_").items, vec!["Research", "Needs review"]);
        assert_eq!(field(&fields, "notion_query_projection_partial").text, "true");
        assert!(!md.contains("notion_Progress_"));
    }
    #[test]
    fn adversarial_values_cannot_inject_frontmatter_fields() {
        let raw=json!({"properties":{
            "Status\nadmin":{"type":"select","select":{"name":"Safe"}},
            "Note":{"type":"rich_text","rich_text":[
                {"plain_text":"safe\nadmin: true"}]},
            "Selector":{"type":"status","status":{"name":"true"}},
            "Hash":{"type":"select","select":{"name":"# hidden"}},
            "Supported":{"type":"checkbox","checkbox":false}
        }});
        let md=for_page(&raw,A);
        let fields=note_fields(&md);
        assert!(fields.iter().any(|f| f.key.starts_with("notion_Supported_")));
        assert!(!fields.iter().any(|f| f.key == "admin"));
        assert!(!fields.iter().any(|f| f.key.starts_with("notion_Note_")));
        assert!(md.contains("notion_query_projection_partial: true"));
        assert!(md.len() <= MAX_FRONTMATTER_BYTES);
    }
    #[test]
    fn same_names_with_distinct_property_ids_keep_unique_keys() {
        let p1=json!({"id":"one","type":"checkbox","checkbox":true});
        let p2=json!({"id":"two","type":"checkbox","checkbox":false});
        let first=key("State",&p1).unwrap();
        let second=key("State",&p2).unwrap();
        assert_ne!(first,second);
        assert!(first.starts_with("notion_State_"));
        assert!(second.starts_with("notion_State_"));
    }
    #[test]
    fn malformed_parent_and_unrenderable_types_are_marked_partial() {
        let raw=json!({"parent":{"data_source_id":"bad"},"properties":{
            "A":{"type":"rollup","rollup":{"number":5}},
            "B":{"type":"relation","relation":[{"id":A}]}
        }});
        let md=for_page(&raw,A);
        assert!(!md.contains("notion_data_source_id:"));
        assert!(md.contains("notion_query_projection_partial: true"));
        assert_eq!(note_fields(&md).len(),2);
    }
}
