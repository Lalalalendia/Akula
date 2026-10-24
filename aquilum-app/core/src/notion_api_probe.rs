//! Opt-in, bounded read-only Notion API probe.
//!
//! The root is an explicitly authorized page. Nothing writes to Akula's vault,
//! creates a persistent index, or asserts that an entire Notion workspace was read.
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use uuid::Uuid;

pub mod http;
pub mod projection;
pub mod publish;
pub mod cached_import; pub mod structured_bundle; pub mod identity_registry; mod typed_frontmatter;
pub mod snapshot;
pub mod refresh;
pub mod sync_policy;
pub const API_VERSION: &str = "2026-03-11";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadRequest {
    Page(String),
    Markdown(String),
    BlockChildren { id: String, cursor: Option<String> },
    Database(String),
    DataSource(String),
    DataSourceRows { id: String, cursor: Option<String> },
    Views { id: String, cursor: Option<String> },
    View(String),
    PageProperty { page: String, property: String, cursor: Option<String> },
}

/// Network authority is limited by the implementation to explicitly enumerated
/// read endpoints (some reads use POST). Mock implementations require no token.
pub trait NotionReadTransport {
    fn read(&mut self, request: &ReadRequest) -> Result<Value, String>;
}

#[derive(Clone, Copy, Debug)]
pub struct ProbeLimits {
    pub max_requests: usize,
    pub max_objects: usize,
    pub max_depth: usize,
    pub max_pages_per_collection: usize,
}
impl Default for ProbeLimits {
    fn default() -> Self {
        Self {
            max_requests: 2_000,
            max_objects: 20_000,
            max_depth: 48,
            max_pages_per_collection: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RawObject {
    pub kind: String,
    pub id: String,
    pub json: Value,
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Serialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    pub kind: String,
    /// Notion property ID; stable across renames (can be URL-encoded, not necessarily UUID).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SourcePreview {
    pub format: &'static str,
    pub api_version: &'static str,
    pub root: String,
    pub workspace_complete: bool,
    pub root_traversal_complete: bool,
    pub requests: usize,
    pub objects: Vec<RawObject>,
    pub links: Vec<Link>,
    pub unresolved_target_ids: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PublicSummary {
    pub format: &'static str,
    pub api_version: &'static str,
    pub root: String,
    pub workspace_complete: bool,
    pub root_traversal_complete: bool,
    pub requests: usize,
    pub counts: BTreeMap<String, usize>,
    pub links: usize,
    /// Count edges by semantics without disclosing private target UUIDs or text.
    pub link_counts: BTreeMap<String, usize>,
    pub unresolved_targets: usize,
    pub warnings: Vec<String>,
}
impl SourcePreview {
    /// This is safe to log when the root UUID is not itself confidential.
    /// Raw page text, backlinks, file URLs, and tokens are never emitted here.
    pub fn summary(&self) -> PublicSummary {
        let mut counts = BTreeMap::new();
        for obj in &self.objects {
            *counts.entry(obj.kind.clone()).or_insert(0) += 1;
        }
        let mut link_counts = BTreeMap::new();
        for link in &self.links {
            *link_counts.entry(link.kind.clone()).or_insert(0) += 1;
        }
        PublicSummary {
            format: self.format,
            api_version: self.api_version,
            root: self.root.clone(),
            workspace_complete: self.workspace_complete,
            root_traversal_complete: self.root_traversal_complete,
            requests: self.requests,
            counts,
            links: self.links.len(),
            link_counts,
            unresolved_targets: self.unresolved_target_ids.len(),
            warnings: self.warnings.clone(),
        }
    }
}

fn canonical_id(s: &str) -> Result<String, String> {
    Uuid::parse_str(s).map(|id| id.to_string())
        .map_err(|_| "Notion ID must be a UUID".to_owned())
}
fn required_id(obj: &Value, type_name: &str) -> Result<String, String> {
    if obj.get("object").and_then(Value::as_str) != Some(type_name) {
        return Err(format!("expected Notion {type_name} object"));
    }
    let id = obj.get("id").and_then(Value::as_str).ok_or("Notion object missing ID")?;
    canonical_id(id)
}

#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq)]
enum TaskKind { Page, Blocks, Database, DataSource }
#[derive(Clone, Debug)]
struct Task { kind: TaskKind, id: String, depth: usize }

pub fn inspect_root<T: NotionReadTransport>(
    transport: &mut T,
    root: &str,
    limits: ProbeLimits,
) -> Result<SourcePreview, String> {
    let root = canonical_id(root)?;
    if limits.max_requests == 0 || limits.max_objects == 0 || limits.max_depth == 0
        || limits.max_pages_per_collection == 0 {
        return Err("all probe budgets must be positive".into());
    }
    let mut scan = Scan {
        transport, limits,
        requests: 0,
        objects: BTreeMap::new(),
        links: BTreeSet::new(),
        warnings: Vec::new(),
        queued: VecDeque::from([Task { kind: TaskKind::Page, id: root.clone(), depth: 0 }]),
        seen: BTreeSet::new(),
    };
    while let Some(task) = scan.queued.pop_front() {
        if task.depth > limits.max_depth {
            return Err("Notion tree exceeds bounded traversal depth".into());
        }
        if !scan.seen.insert((task.kind.clone(), task.id.clone())) {
            continue;
        }
        match task.kind {
            TaskKind::Page => scan.page(&task.id, task.depth)?,
            TaskKind::Blocks => scan.blocks(&task.id, task.depth)?,
            TaskKind::Database => scan.database(&task.id, task.depth)?,
            TaskKind::DataSource => scan.data_source(&task.id, task.depth)?,
        }
    }

    let known: BTreeSet<String> = scan.objects.keys().map(|(_, id)| id.clone()).collect();
    let unresolved_target_ids: Vec<String> = scan.links.iter()
        .filter(|edge| !known.contains(&edge.to))
        .map(|edge| edge.to.clone())
        .collect::<BTreeSet<_>>().into_iter().collect();

    Ok(SourcePreview {
        format: "akula_notion_readonly_root_probe_01",
        api_version: API_VERSION,
        root,
        workspace_complete: false,
        root_traversal_complete: scan.warnings.is_empty() && unresolved_target_ids.is_empty(),
        requests: scan.requests,
        objects: scan.objects.into_iter().map(|((kind, id), json)| RawObject {kind, id, json}).collect(),
        links: scan.links.into_iter().collect(),
        unresolved_target_ids,
        warnings: scan.warnings,
    })
}

struct Scan<'a, T: NotionReadTransport> {
    transport: &'a mut T,
    limits: ProbeLimits,
    requests: usize,
    objects: BTreeMap<(String, String), Value>,
    links: BTreeSet<Link>,
    warnings: Vec<String>,
    queued: VecDeque<Task>,
    seen: BTreeSet<(TaskKind, String)>,
}
impl<T: NotionReadTransport> Scan<'_, T> {
    fn request(&mut self, req: ReadRequest) -> Result<Value, String> {
        if self.requests >= self.limits.max_requests {
            return Err("Notion request budget exceeded; no complete snapshot".into());
        }
        self.requests += 1;
        self.transport.read(&req)
    }
    fn insert(&mut self, kind: &str, id: String, json: Value) -> Result<(), String> {
        let key = (kind.to_owned(), id);
        if !self.objects.contains_key(&key) && self.objects.len() >= self.limits.max_objects {
            return Err("Notion object budget exceeded".into());
        }
        self.objects.insert(key, json);
        Ok(())
    }
    fn edge(&mut self, from: &str, to: &str, kind: &str) -> Result<(), String> {
        self.edge_with_property(from, to, kind, None)
    }
    fn edge_with_property(&mut self, from: &str, to: &str, kind: &str,
        property_id: Option<&str>) -> Result<(), String> {
        if self.links.len() >= self.limits.max_objects * 16 {
            return Err("Notion edge budget exceeded".into());
        }
        self.links.insert(Link {
            from: from.to_owned(), to: canonical_id(to)?, kind: kind.to_owned(),
            property_id: property_id.map(str::to_owned),
        });
        Ok(())
    }
    fn queue(&mut self, kind: TaskKind, id: &str, depth: usize) -> Result<(), String> {
        if depth > self.limits.max_depth { return Err("Notion tree exceeds depth limit".into()); }
        let id = canonical_id(id)?;
        if self.queued.len() >= self.limits.max_objects {
            return Err("Notion pending task budget exceeded".into());
        }
        self.queued.push_back(Task { kind, id, depth });
        Ok(())
    }

    /// Fail closed on malformed/looping cursors, partial result windows or missing
    /// status. No partial batch is ever announced as complete.
    fn collect_list<F>(&mut self, mut build: F, mut initial: Option<Value>) -> Result<Vec<Value>, String>
    where F: FnMut(Option<String>) -> ReadRequest {
        let mut records = Vec::new();
        let mut seen_cursors = BTreeSet::new();
        let mut cursor = None;
        for _ in 0..self.limits.max_pages_per_collection {
            let response = if let Some(first) = initial.take() { first } else { self.request(build(cursor.clone()))? };
            if response.get("object").and_then(Value::as_str) != Some("list") {
                return Err("Notion list response has unexpected object type".into());
            }
            if let Some(status) = response.get("request_status") {
                if status.get("type").and_then(Value::as_str) != Some("complete") {
                    return Err("Notion returned an incomplete list window".into());
                }
            }
            let batch = response.get("results").and_then(Value::as_array)
                .ok_or("Notion list missing results array")?;
            if batch.len() > 100 {
                return Err("Notion list has more than 100 items in one page".into());
            }
            if records.len().checked_add(batch.len()).ok_or("list counter overflow")? > self.limits.max_objects {
                return Err("Notion list object budget exceeded".into());
            }
            records.extend(batch.iter().cloned());
            let more = response.get("has_more").and_then(Value::as_bool)
                .ok_or("Notion list missing boolean has_more")?;
            if !more {
                return Ok(records);
            }
            let next = response.get("next_cursor").and_then(Value::as_str)
                .filter(|s| !s.is_empty()).ok_or("Notion has_more without cursor")?;
            if !seen_cursors.insert(next.to_owned()) {
                return Err("Notion returned a repeating cursor".into());
            }
            cursor = Some(next.to_owned());
        }
        Err("Notion collection page limit exceeded".into())
    }
    fn page(&mut self, id: &str, depth: usize) -> Result<(), String> {
        let obj = self.request(ReadRequest::Page(id.to_owned()))?;
        if required_id(&obj, "page")? != id { return Err("Notion page ID mismatch".into()); }
        // Relations are primary source data. Missing IDs must fail closed:
        // silently skipping a relation would falsely certify a complete graph.
        let properties = obj.get("properties").and_then(Value::as_object)
            .ok_or("Notion page missing properties object")?;
        let mut relations = Vec::new();
        for prop in properties.values() {
            if prop.get("type").and_then(Value::as_str) == Some("relation") {
                let pid = prop.get("id").and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or("Notion relation property missing stable ID")?;
                relations.push(pid.to_owned());
            }
        }
        scan_mentions(&obj, id, &mut self.links, self.limits.max_objects * 16)?;
        self.insert("page", id.to_owned(), obj)?;
        for property in relations {
            let items = self.property_items(id, &property)?;
            for item in items {
                if item.get("type").and_then(Value::as_str) != Some("relation") {
                    return Err("Notion relation property item has unexpected type".into());
                }
                let target = item.get("relation").and_then(|o| o.get("id"))
                    .and_then(Value::as_str).filter(|id| !id.is_empty())
                    .ok_or("Notion relation item missing target ID")?;
                self.edge_with_property(id, target, "relation", Some(&property))?;
            }
        }

        let markdown = self.request(ReadRequest::Markdown(id.to_owned()))?;
        // API 2026-03-11 includes object and page identity. Do not accept
        // arbitrary JSON (or another page's body) as a complete snapshot.
        if required_id(&markdown, "page_markdown")? != id {
            return Err("Notion markdown page ID mismatch".into());
        }
        if markdown.get("markdown").and_then(Value::as_str).is_none() {
            return Err("Notion markdown response missing text".into());
        }
        if markdown.get("truncated") != Some(&Value::Bool(false)) {
            self.warnings.push(format!("page {id}: markdown is truncated or unverified"));
        }
        if markdown.get("unknown_block_ids").and_then(Value::as_array)
            .is_none_or(|a| !a.is_empty()) {
            self.warnings.push(format!("page {id}: unknown/not-rendered markdown blocks"));
        }
        // GET /pages/{id}/markdown does not require a warnings field.
        // The optional field is currently documented for Markdown updates.
        if let Some(raw_warnings) = markdown.get("warnings") {
            let warnings = raw_warnings.as_array()
                .ok_or("Notion markdown warnings must be an array when present")?;
            if !warnings.is_empty() {
                self.warnings.push(format!("page {id}: markdown parser warnings"));
            }
        }
        self.insert("markdown", id.to_owned(), markdown)?;
        self.queue(TaskKind::Blocks, id, depth + 1)
    }
    fn property_items(&mut self, page: &str, property: &str) -> Result<Vec<Value>, String> {
        let first = self.request(ReadRequest::PageProperty {
            page: page.to_owned(), property: property.to_owned(), cursor: None,
        })?;
        if first.get("object").and_then(Value::as_str) == Some("property_item") {
            self.insert("property_item", format!("{page}:{property}"), first.clone())?;
            return Ok(vec![first]);
        }
        // The first list page is reused rather than fetching it a second time.
        let first_page = Some(first);
        let items = self.collect_list(|cursor| ReadRequest::PageProperty {
            page: page.to_owned(), property: property.to_owned(), cursor,
        }, first_page)?;
        self.insert("property_item", format!("{page}:{property}"), serde_json::json!(items))?;
        Ok(items)
    }
    fn blocks(&mut self, id: &str, depth: usize) -> Result<(), String> {
        let blocks = self.collect_list(|cursor| ReadRequest::BlockChildren { id: id.to_owned(), cursor }, None)?;
        for block in blocks {
            let block_id = required_id(&block, "block")?;
            self.edge(id, &block_id, "contains")?;
            let typ = block.get("type").and_then(Value::as_str).unwrap_or("");
            self.insert("block", block_id.clone(), block.clone())?;
            scan_mentions(&block, &block_id, &mut self.links, self.limits.max_objects * 16)?;
            if typ == "synced_block" {
                if let Some(original_id) = block.pointer("/synced_block/synced_from/block_id")
                    .and_then(Value::as_str) {
                    self.edge(&block_id, original_id, "synced_from")?;
                }
            }
            match typ {
                "child_page" => self.queue(TaskKind::Page, &block_id, depth + 1)?,
                "child_database" => self.queue(TaskKind::Database, &block_id, depth + 1)?,
                _ if block.get("has_children").and_then(Value::as_bool) == Some(true) =>
                    self.queue(TaskKind::Blocks, &block_id, depth + 1)?,
                _ => {}
            }
        }
        Ok(())
    }
    fn database(&mut self, id: &str, depth: usize) -> Result<(), String> {
        let db = self.request(ReadRequest::Database(id.to_owned()))?;
        if required_id(&db, "database")? != id { return Err("Notion database ID mismatch".into()); }
        let data_sources = db.get("data_sources").and_then(Value::as_array)
            .ok_or("Notion database missing data_sources")?.clone();
        self.insert("database", id.to_owned(), db)?;
        for source in data_sources {
            let source_id = canonical_id(source.get("id").and_then(Value::as_str)
                .ok_or("Notion data source missing ID")?)?;
            self.edge(id, &source_id, "data_source")?;
            self.queue(TaskKind::DataSource, &source_id, depth + 1)?;
        }
        let views = self.collect_list(|cursor| ReadRequest::Views { id: id.to_owned(), cursor }, None)?;
        for view in views {
            let view_id = required_id(&view, "view")?;
            let full = self.request(ReadRequest::View(view_id.clone()))?;
            if required_id(&full, "view")? != view_id {
                return Err("Notion view ID mismatch".into());
            }
            self.insert("view", view_id.clone(), full)?;
            self.edge(id, &view_id, "view")?;
        }
        Ok(())
    }
    fn data_source(&mut self, id: &str, depth: usize) -> Result<(), String> {
        let source = self.request(ReadRequest::DataSource(id.to_owned()))?;
        if required_id(&source, "data_source")? != id {
            return Err("Notion data source ID mismatch".into());
        }
        self.insert("data_source", id.to_owned(), source)?;
        let rows = self.collect_list(|cursor| ReadRequest::DataSourceRows { id: id.to_owned(), cursor }, None)?;
        if rows.len() >= 10_000 {
            return Err("Notion data source hit 10,000-row query boundary; partitioning not implemented".into());
        }
        for row in rows {
            let page_id = required_id(&row, "page")?;
            self.edge(id, &page_id, "row")?;
            self.insert("page", page_id.clone(), row)?;
            self.queue(TaskKind::Page, &page_id, depth + 1)?;
        }
        Ok(())
    }
}

fn scan_mentions(value: &Value, source: &str, edges: &mut BTreeSet<Link>, limit: usize)
    -> Result<(), String> {
    fn walk(value: &Value, source: &str, edges: &mut BTreeSet<Link>,
        budget: usize, depth: usize) -> Result<(), String> {
        if depth > 64 { return Err("Notion embedded JSON exceeds parse depth".into()); }
        match value {
            Value::Object(map) => {
                if map.get("type").and_then(Value::as_str) == Some("mention") {
                    if let Some(target) = map.get("mention").and_then(|v| {
                        let k = v.get("type")?.as_str()?;
                        if k == "page" || k == "database" {
                            v.get(k)?.get("id")?.as_str()
                        } else { None }
                    }) {
                        if edges.len() >= budget { return Err("Notion mention edge budget exceeded".into()); }
                        edges.insert(Link { from: source.to_owned(), to: canonical_id(target)?,
                            kind: "mention".into(), property_id: None });
                    }
                }
                for value in map.values() { walk(value, source, edges, budget, depth + 1)?; }
            }
            Value::Array(items) => {
                for value in items { walk(value, source, edges, budget, depth + 1)?; }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, source, edges, limit, 0)
}

#[cfg(test)]
mod tests;
