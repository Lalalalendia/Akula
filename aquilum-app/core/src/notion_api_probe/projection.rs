//! Initial, read-only projection of a Notion snapshot into Akula note candidates.
//! This step NEVER writes files. The owning host must later use Core File Gate,
//! after the operator explicitly approves a target workspace and the preview.
use super::{SourcePreview, Link};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct ImportCandidate {
    pub source_id: String,
    pub relative_path: String,
    pub markdown: String,
    pub relations: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ImportPreview {
    pub source_root: String,
    pub candidates: Vec<ImportCandidate>,
    pub unresolved: Vec<String>,
    pub publish_allowed: bool,
}

fn safe_title(value: &Value) -> String {
    let title = value.get("properties")
        .and_then(Value::as_object)
        .and_then(|props| props.values().find(|v| v.get("type").and_then(Value::as_str) == Some("title")))
        .and_then(|prop| prop.get("title")).and_then(Value::as_array)
        .map(|runs| runs.iter().filter_map(|run| run.get("plain_text").and_then(Value::as_str))
            .collect::<String>())
        .filter(|v| !v.is_empty());
    title.unwrap_or_else(|| "Notion page".into())
        .chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

pub fn prepare_import(source: &SourcePreview) -> Result<ImportPreview, String> {
    // A failed traversal is not a backup or a publishable import.
    let ids: BTreeSet<_> = source.objects.iter()
        .filter(|o| o.kind == "page").map(|o| o.id.clone()).collect();
    let mut content = BTreeMap::new();
    for obj in &source.objects {
        if obj.kind == "markdown" {
            let text = obj.json.get("markdown").and_then(Value::as_str)
                .ok_or("Markdown object lacks text")?;
            if content.insert(obj.id.clone(), text.to_owned()).is_some() {
                return Err("duplicate Markdown identity".into());
            }
        }
    }
    let mut candidates = Vec::new();
    for obj in source.objects.iter().filter(|o| o.kind == "page") {
        if !content.contains_key(&obj.id) {
            return Err("page missing Markdown snapshot".into());
        }
        let id = uuid::Uuid::parse_str(&obj.id).map_err(|_| "invalid source page ID")?.to_string();
        let relations: Vec<String> = source.links.iter()
            .filter(|edge| edge.kind == "relation" && edge.from == id)
            .map(|edge| edge.to.clone()).collect();
        let mut markdown = String::new();
        markdown.push_str(&format!("# {}\n\n",safe_title(&obj.json)));
        markdown.push_str(content.get(&obj.id).unwrap());
        markdown.push_str("\n\n");
        // Keep stable Notion IDs; don't rely on page names for identity.
        if !relations.is_empty() {
            markdown.push_str("## Related Notion objects\n\n");
            for target in &relations {
                if ids.contains(target) {
                    markdown.push_str(&format!("- [Related](<{}.md>)\n",target));
                } else {
                    markdown.push_str(&format!("- Unresolved relation: {}\n",target));
                }
            }
        }
        candidates.push(ImportCandidate {
            source_id:id.clone(),
            relative_path: format!("Notion-import/{id}.md"),
            markdown,
            relations,
        });
    }
    let mut unresolved: BTreeSet<String> = source.unresolved_target_ids.iter().cloned().collect();
    for Link { kind, to, .. } in &source.links {
        if kind == "relation" && !ids.contains(to) { unresolved.insert(to.clone()); }
    }
    Ok(ImportPreview {
        source_root:source.root.clone(),
        candidates,
        unresolved:unresolved.iter().cloned().collect(),
        publish_allowed:source.root_traversal_complete && unresolved.is_empty() && !source.objects.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notion_api_probe::{RawObject, SourcePreview, API_VERSION};
    use serde_json::json;
    #[test]
    fn two_rows_keep_ids_and_relation_as_local_link() {
        let a="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let b="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        let page=|id:&str,title:&str| RawObject{kind:"page".into(),id:id.into(),
            json:json!({"properties":{"Name":{"type":"title","title":[{"plain_text":title}]}}})};
        let md=|id:&str,t:&str|RawObject{kind:"markdown".into(),id:id.into(),json:json!({"markdown":t})};
        let source=SourcePreview{format:"test",api_version:API_VERSION,root:a.into(),
            workspace_complete:false,root_traversal_complete:true,requests:1,
            objects:vec![page(a,"Same"),page(b,"Same"),md(a,"A content"),md(b,"B content")],
            links:vec![Link{from:a.into(),to:b.into(),kind:"relation".into(),property_id:Some("rel".into())}],
            unresolved_target_ids:vec![],warnings:vec![]};
        let result=prepare_import(&source).unwrap();
        assert!(result.publish_allowed);
        assert_eq!(result.candidates.len(),2);
        assert_ne!(result.candidates[0].relative_path,result.candidates[1].relative_path);
        assert!(result.candidates[0].markdown.contains(b));
    }
    #[test]
    fn partial_snapshot_cannot_publish() {
        let source=SourcePreview{format:"test",api_version:API_VERSION,
            root:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),workspace_complete:false,
            root_traversal_complete:false,requests:0,objects:vec![],links:vec![],
            unresolved_target_ids:vec!["bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into()],warnings:vec!["incomplete".into()]};
        assert!(!prepare_import(&source).unwrap().publish_allowed);
    }
}
