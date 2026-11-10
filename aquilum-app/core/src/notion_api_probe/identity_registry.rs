//! Root-scoped, stable Notion -> Akula identity registry.
//!
//! Independent, create-only source-identity metadata, NOT editable Bases. The
//! verified Notion root is the only available scope in this API contract:
//! workspace identity is unverified, so cross-root merging is forbidden.
//! Missing endpoints never become fabricated backlinks or writable relations.
use super::{Link, RawObject, SourcePreview, snapshot::SnapshotStore};
use crate::{app_core::Core, files::gate, history::Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use uuid::Uuid;

const FORMAT: &str = "akula_notion_root_identity_registry_v1";
const MAX_BYTES: usize = 32 * 1024 * 1024;
const DIR: &str = "Notion-import";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Node {
    kind: String,
    source_id: String,
    akula_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SchemaProperty {
    owner_id: String,
    owner_kind: String,
    source_property_id: String,
    akula_id: String,
    source_type: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct RowProperty {
    page_id: String,
    source_property_id: Option<String>,
    schema_akula_id: Option<String>,
    source_type: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Resolution { Resolved, Missing, Ambiguous }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Edge {
    from: String,
    to: String,
    kind: String,
    source_property_id: Option<String>,
    from_akula_id: Option<String>,
    to_akula_id: Option<String>,
    resolution: Resolution,
}
#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    format: String,
    provider: String,
    source_scope_kind: String,
    source_root: String,
    source_digest: String,
    workspace_identity_verified: bool,
    cross_root_merge_supported: bool,
    editable_bases_restored: bool,
    nodes: Vec<Node>,
    schema_properties: Vec<SchemaProperty>,
    row_properties: Vec<RowProperty>,
    edges: Vec<Edge>,
    duplicate_source_edges: usize,
    unidentified_schema_properties: usize,
    unresolved_source_targets: usize,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    payload: Payload,
    payload_blake3: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct RegistryReview {
    pub source_root: String,
    pub source_digest: String,
    pub relative_path: String,
    pub payload_blake3: String,
    pub bytes: usize,
    pub node_count: usize,
    pub schema_property_count: usize,
    pub row_property_count: usize,
    pub unmatched_row_property_count: usize,
    pub resolved_edge_count: usize,
    pub unresolved_edge_count: usize,
    pub duplicate_source_edges: usize,
    pub unidentified_schema_property_count: usize,
    pub unresolved_source_target_count: usize,
    pub workspace_identity_verified: bool,
    pub cross_root_merge_supported: bool,
    pub editable_bases_restored: bool,
}
pub struct PreparedRegistry {
    bytes: Vec<u8>,
    review: RegistryReview,
}
impl PreparedRegistry {
    pub fn review(&self) -> &RegistryReview { &self.review }
}
#[derive(Serialize)]
pub struct RegistryReceipt {
    pub relative_path: String,
    pub already_identical: bool,
    pub payload_blake3: String,
    pub unresolved_edge_count: usize,
    pub unmatched_row_property_count: usize,
    pub unidentified_schema_property_count: usize,
    pub unresolved_source_target_count: usize,
    pub editable_bases_restored: bool,
}

fn canonical_uuid(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}
fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest.bytes().all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}
fn valid_source_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}
/// Length-delimited domain separation; no source names or paths are identities.
fn stable(scope: &str, parts: &[&str]) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"akula:notion:root-scoped-id:v1");
    for part in std::iter::once(scope).chain(parts.iter().copied()) {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("akn_{}", hash.finalize().to_hex())
}
pub(super) fn node_id(root: &str, kind: &str, id: &str) -> String {
    stable(root, &["node", kind, id])
}
pub(super) fn property_id(root: &str, owner_kind: &str, owner: &str, id: &str) -> String {
    stable(root, &["property", owner_kind, owner, id])
}
fn relative_path(root: &str, digest: &str) -> String {
    format!("{DIR}/{root}-{digest}.notion-identities.json")
}
fn property_key(kind: &str, owner: &str, id: &str) -> (String, String, String) {
    (kind.to_owned(), owner.to_owned(), id.to_owned())
}
fn endpoint(index: &BTreeMap<String, Vec<String>>, id: &str) -> (Option<String>, Resolution) {
    match index.get(id).map(Vec::as_slice) {
        Some([single]) => (Some(single.clone()), Resolution::Resolved),
        Some([]) | None => (None, Resolution::Missing),
        Some(_) => (None, Resolution::Ambiguous),
    }
}
fn relation(index: &BTreeMap<String, Vec<String>>, link: &Link) -> Edge {
    let (from_akula_id, from_resolution) = endpoint(index, &link.from);
    let (to_akula_id, to_resolution) = endpoint(index, &link.to);
    let resolution = if from_resolution == Resolution::Ambiguous
        || to_resolution == Resolution::Ambiguous
    { Resolution::Ambiguous } else if from_resolution == Resolution::Missing
        || to_resolution == Resolution::Missing
    { Resolution::Missing } else { Resolution::Resolved };
    Edge {
        from:link.from.clone(), to:link.to.clone(), kind:link.kind.clone(),
        source_property_id:link.property_id.clone(),
        from_akula_id, to_akula_id, resolution,
    }
}
fn parent_link(object: &RawObject) -> Option<Link> {
    let parent = object.json.get("parent")?;
    let kind = parent.get("type")?.as_str()?;
    if !matches!(kind, "page_id" | "data_source_id" | "database_id" | "block_id") {
        return None;
    }
    let id = parent.get(kind)?.as_str()?;
    if !valid_source_id(id) { return None; }
    Some(Link {
        from:object.id.clone(), to:id.to_owned(),
        kind:format!("parent_{kind}"), property_id:None,
    })
}
fn normalize(source: &SourcePreview, digest: &str) -> Result<PreparedRegistry, String> {
    let root = &source.root;
    if !canonical_uuid(root) || !valid_digest(digest) || !source.root_traversal_complete
        || source.workspace_complete
    { return Err("Notion root identity or traversal is not verified".into()); }
    let mut nodes = Vec::new();
    let mut identities = BTreeSet::new();
    let mut index: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut properties = Vec::new();
    let mut property_index = BTreeMap::new();
    let mut row_properties = Vec::new();
    let mut unidentified_schema_properties = 0_usize;
    // First pass: register ALL actual entities before resolving any edge.
    for object in &source.objects {
        if object.kind == "markdown" { continue; } // content of the page, not a second row
        if object.kind.is_empty() || object.kind.len() > 64
            || !valid_source_id(&object.id)
            || !identities.insert((&object.kind, &object.id))
        { return Err("duplicate or invalid Notion source entity identity".into()); }
        let akula_id = node_id(root, &object.kind, &object.id);
        nodes.push(Node {kind:object.kind.clone(),source_id:object.id.clone(),
            akula_id:akula_id.clone()});
        index.entry(object.id.clone()).or_default().push(akula_id);
    }
    if nodes.is_empty() { return Err("Notion identity registry has no entities".into()); }
    // Schema property IDs are scoped to an owner, not their mutable display names.
    for object in &source.objects {
        if object.kind != "data_source" && object.kind != "database" { continue; }
        let Some(raw) = object.json.get("properties").and_then(Value::as_object) else { continue };
        for value in raw.values() {
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                // Original type/value is still in the full JSON bundle, but
                // this schema field cannot be assigned a stable Akula ID.
                unidentified_schema_properties += 1;
                continue;
            };
            if !valid_source_id(id) { return Err("unsafe Notion schema property ID".into()); }
            let key = property_key(&object.kind, &object.id, id);
            if property_index.contains_key(&key) {
                return Err("duplicate schema property ID in one Notion source".into());
            }
            let akula_id = property_id(root, &object.kind, &object.id, id);
            property_index.insert(key, akula_id.clone());
            properties.push(SchemaProperty {
                owner_id:object.id.clone(), owner_kind:object.kind.clone(),
                source_property_id:id.to_owned(), akula_id,
                source_type:value.get("type").and_then(Value::as_str)
                    .unwrap_or("unknown").to_owned(),
            });
        }
    }
    // Page values bind to existing schema IDs only when the source owner and
    // field IDs match. Unknown properties remain preserved in the raw bundle.
    for object in &source.objects {
        if object.kind != "page" { continue; }
        let parent = object.json.get("parent");
        let owner = parent.and_then(|p| p.get("type")).and_then(Value::as_str)
            .and_then(|kind| match kind {
                "data_source_id" => Some(("data_source", "data_source_id")),
                "database_id" => Some(("database", "database_id")),
                _ => None,
            }).and_then(|(kind, field)| parent?.get(field)?.as_str().map(|id| (kind,id)));
        let Some(values) = object.json.get("properties").and_then(Value::as_object) else { continue };
        for property in values.values() {
            let id = property.get("id").and_then(Value::as_str);
            if id.is_some_and(|id| !valid_source_id(id)) {
                return Err("unsafe Notion row property ID".into());
            }
            let bound = owner.and_then(|(kind, owner)| id.and_then(|id|
                property_index.get(&property_key(kind, owner, id)))).cloned();
            row_properties.push(RowProperty {
                page_id:object.id.clone(), source_property_id:id.map(str::to_owned),
                schema_akula_id:bound,
                source_type:property.get("type").and_then(Value::as_str)
                    .unwrap_or("unknown").to_owned(),
            });
        }
    }
    let mut raw_edges = BTreeSet::new();
    let mut duplicate_source_edges = 0_usize;
    for link in source.links.iter().cloned().chain(
        source.objects.iter().filter_map(parent_link)
    ) {
        if !valid_source_id(&link.from) || !valid_source_id(&link.to)
            || link.kind.is_empty() || link.kind.len() > 128
            || link.property_id.as_deref().is_some_and(|id| !valid_source_id(id))
        { return Err("unsafe Notion relation edge identity".into()); }
        if !raw_edges.insert(link) { duplicate_source_edges += 1; }
    }
    let mut edges = raw_edges.iter().map(|link| relation(&index,link)).collect::<Vec<_>>();
    nodes.sort_by(|a,b| (&a.kind,&a.source_id).cmp(&(&b.kind,&b.source_id)));
    properties.sort_by(|a,b| (&a.owner_kind,&a.owner_id,&a.source_property_id)
        .cmp(&(&b.owner_kind,&b.owner_id,&b.source_property_id)));
    row_properties.sort_by(|a,b| (&a.page_id,&a.source_property_id,&a.source_type)
        .cmp(&(&b.page_id,&b.source_property_id,&b.source_type)));
    edges.sort_by(|a,b| (&a.from,&a.to,&a.kind,&a.source_property_id)
        .cmp(&(&b.from,&b.to,&b.kind,&b.source_property_id)));
    let payload = Payload {
        format:FORMAT.into(),provider:"notion".into(),source_scope_kind:"root".into(),
        source_root:root.to_owned(),source_digest:digest.to_owned(),
        workspace_identity_verified:false,cross_root_merge_supported:false,
        editable_bases_restored:false,nodes,schema_properties:properties,
        row_properties,edges,duplicate_source_edges,
        unidentified_schema_properties,
        unresolved_source_targets:source.unresolved_target_ids.iter().collect::<BTreeSet<_>>().len(),
    };
    encode(payload)
}
fn review(payload: &Payload, digest: String, bytes: usize) -> RegistryReview {
    RegistryReview {
        source_root:payload.source_root.clone(),
        source_digest:payload.source_digest.clone(),
        relative_path:relative_path(&payload.source_root,&payload.source_digest),
        payload_blake3:digest, bytes, node_count:payload.nodes.len(),
        schema_property_count:payload.schema_properties.len(),
        row_property_count:payload.row_properties.len(),
        unmatched_row_property_count:payload.row_properties.iter()
            .filter(|f| f.schema_akula_id.is_none()).count(),
        resolved_edge_count:payload.edges.iter()
            .filter(|e| e.resolution == Resolution::Resolved).count(),
        unresolved_edge_count:payload.edges.iter()
            .filter(|e| e.resolution != Resolution::Resolved).count(),
        duplicate_source_edges:payload.duplicate_source_edges,
        unidentified_schema_property_count:payload.unidentified_schema_properties,
        unresolved_source_target_count:payload.unresolved_source_targets,
        workspace_identity_verified:false,cross_root_merge_supported:false,
        editable_bases_restored:false,
    }
}
fn encode(payload: Payload) -> Result<PreparedRegistry, String> {
    let digest = blake3::hash(&serde_json::to_vec(&payload)
        .map_err(|_| "cannot encode Notion identity registry payload")?).to_hex().to_string();
    let bytes = serde_json::to_vec(&Envelope{payload,payload_blake3:digest})
        .map_err(|_| "cannot serialize independent Notion identity registry")?;
    let review = verify_bytes(&bytes)?;
    Ok(PreparedRegistry {bytes,review})
}
fn verify_bytes(bytes: &[u8]) -> Result<RegistryReview, String> {
    if bytes.len() > MAX_BYTES { return Err("Notion identity registry too large".into()); }
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| "invalid Notion identity registry JSON")?;
    let p = &envelope.payload;
    if p.format != FORMAT || p.provider != "notion" || p.source_scope_kind != "root"
        || !canonical_uuid(&p.source_root) || !valid_digest(&p.source_digest)
        || p.workspace_identity_verified || p.cross_root_merge_supported
        || p.editable_bases_restored || !valid_digest(&envelope.payload_blake3)
        || p.nodes.is_empty()
    { return Err("unsupported Notion identity registry metadata".into()); }
    let canonical = serde_json::to_vec(p)
        .map_err(|_| "cannot verify Notion identity registry")?;
    let digest = blake3::hash(&canonical).to_hex().to_string();
    if digest != envelope.payload_blake3 {
        return Err("Notion identity registry checksum mismatch".into());
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for node in &p.nodes {
        if node.akula_id != node_id(&p.source_root,&node.kind,&node.source_id)
            || !ids.insert(node.akula_id.clone())
            || !names.insert((&node.kind,&node.source_id))
        { return Err("Notion identity registry node mismatch".into()); }
    }
    for e in &p.edges {
        if e.resolution == Resolution::Resolved &&
            (!e.from_akula_id.as_ref().is_some_and(|id| ids.contains(id))
                || !e.to_akula_id.as_ref().is_some_and(|id| ids.contains(id)))
        { return Err("resolved Notion edge has an absent source node".into()); }
        if e.resolution != Resolution::Resolved
            && e.from_akula_id.is_some() && e.to_akula_id.is_some()
        { return Err("unresolved Notion edge looks resolved".into()); }
    }
    Ok(review(p,digest,bytes.len()))
}
/// Token-free, exact-snapshot normalization. Never writes a user workspace.
pub fn prepare_from_cached(path: &Path, root: &str, digest: &str)
    -> Result<PreparedRegistry, String>
{
    if !canonical_uuid(root) || !valid_digest(digest) {
        return Err("invalid Notion root or digest for identities".into());
    }
    let store = SnapshotStore::open_read_only(path)?;
    let cached = store.load(root)?.ok_or("verified Notion root is unavailable")?;
    if cached.info.digest != digest {
        return Err("Notion snapshot changed after identity review".into());
    }
    normalize(&cached.into_preview(),digest)
}
/// Inspect independent metadata even after the API token and cache are removed.
pub fn inspect_existing(path: &Path) -> Result<RegistryReview, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| "cannot inspect independent Notion identities")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES as u64 {
        return Err("unsafe independent Notion identity file".into());
    }
    let bytes = fs::read(path).map_err(|_| "cannot read independent Notion identities")?;
    let review = verify_bytes(&bytes)?;
    if path.file_name() != Path::new(&review.relative_path).file_name() {
        return Err("Notion identity registry filename does not match root digest".into());
    }
    Ok(review)
}
/// Publish a standalone registry ONLY to a uniquely registered, approved Vault.
/// Existing content, even an earlier local edit, is never overwritten.
pub fn publish_approved(core: &Core, workspace: &Path, prepared: &PreparedRegistry,
    approved: bool) -> Result<RegistryReceipt, String>
{
    if !approved { return Err("Notion identity publication requires approval".into()); }
    let verified = verify_bytes(&prepared.bytes)?;
    if verified.payload_blake3 != prepared.review.payload_blake3 {
        return Err("Notion identity registry was modified before publication".into());
    }
    let root = fs::canonicalize(workspace).map_err(|_| "invalid Akula Vault")?;
    if !root.is_dir() { return Err("Akula Vault is not a directory".into()); }
    let registrations = core.ui_state.workspace_roots()
        .map_err(|_| "cannot verify registered Akula Vaults")?;
    if registrations.iter().filter(|path| fs::canonicalize(path).ok()
        .as_deref() == Some(root.as_path())).count() != 1
    { return Err("selected Akula Vault is not uniquely registered".into()); }
    let folder = root.join(DIR);
    let meta = fs::symlink_metadata(&folder)
        .map_err(|_| "Notion import folder must first be created via File Gate")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("unsafe Notion import directory".into());
    }
    let output = root.join(&verified.relative_path);
    let identical = match fs::symlink_metadata(&output) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink()
            && meta.len() <= MAX_BYTES as u64 =>
        {
            let existing = fs::read(&output)
                .map_err(|_| "cannot read previous Notion identity registry")?;
            if existing != prepared.bytes {
                return Err("existing Notion identity registry differs; no overwrite".into());
            }
            true
        }
        Ok(_) => return Err("unsafe existing Notion identity registry target".into()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let json = std::str::from_utf8(&prepared.bytes)
                .map_err(|_| "Notion identity JSON must be UTF-8")?;
            gate::create(core,&output,json,Source::External,Some("Notion identity registry"))
                .map_err(|_| "File Gate refused create-only identity registry")?;
            false
        }
        Err(_) => return Err("cannot inspect identity registry target".into()),
    };
    let stored = fs::read(&output).map_err(|_| "cannot verify identity registry readback")?;
    if stored != prepared.bytes ||
        verify_bytes(&stored)?.payload_blake3 != verified.payload_blake3
    { return Err("Notion identity registry readback failed".into()); }
    Ok(RegistryReceipt {
        relative_path:verified.relative_path, already_identical:identical,
        payload_blake3:verified.payload_blake3,
        unresolved_edge_count:verified.unresolved_edge_count,
        unmatched_row_property_count:verified.unmatched_row_property_count,
        unidentified_schema_property_count:verified.unidentified_schema_property_count,
        unresolved_source_target_count:verified.unresolved_source_target_count,
        editable_bases_restored:false,
    })
}
#[cfg(test)]
#[path="identity_registry_tests.rs"]
mod tests;
