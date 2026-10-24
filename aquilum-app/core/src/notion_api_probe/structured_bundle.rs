//! Independent, create-only JSON preservation of verified Notion API objects.
//!
//! This is a lossless representation of what the selected Notion API root
//! returned (not proof of workspace completeness). The bundle is portable
//! without the Notion token or Akula's read-only ui-state cache. It is NOT a
//! native Bases implementation, downloaded media, or a scheduled sync writer.
use super::{Link, RawObject, API_VERSION, snapshot::SnapshotStore};
use crate::{app_core::Core, files::gate, history::Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const FORMAT: &str = "akula_notion_structured_bundle_v1";
const IMPORT_DIR: &str = "Notion-import";
const MAX_BUNDLE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct PreservedObject {
    kind: String,
    id: String,
    /// Unchanged parsed Notion API JSON including future/unknown property types.
    json: Value,
}
#[derive(Serialize, Deserialize)]
struct PreservedEdge {
    from: String,
    to: String,
    kind: String,
    property_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Payload {
    format: String,
    api_version: String,
    root: String,
    source_digest: String,
    workspace_complete: bool,
    root_traversal_complete: bool,
    native_database_views_restored: bool,
    media_bytes_downloaded: bool,
    objects: Vec<PreservedObject>,
    edges: Vec<PreservedEdge>,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    payload: Payload,
    payload_blake3: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BundleReview {
    pub root: String,
    pub source_digest: String,
    pub payload_blake3: String,
    pub relative_path: String,
    pub bytes: usize,
    pub object_counts: BTreeMap<String, usize>,
    pub property_types: BTreeMap<String, usize>,
    pub edge_count: usize,
    pub workspace_complete: bool,
    pub native_database_views_restored: bool,
    pub media_bytes_downloaded: bool,
}
pub struct PreparedBundle {
    bytes: Vec<u8>,
    review: BundleReview,
}
impl PreparedBundle {
    pub fn review(&self) -> &BundleReview { &self.review }
}

/// Metadata-only receipt: never includes personal Notion content or file URLs.
#[derive(Debug, Serialize)]
pub struct BundleReceipt {
    pub relative_path: String,
    pub already_identical: bool,
    pub verified_bytes: usize,
    pub payload_blake3: String,
    pub native_database_views_restored: bool,
    pub media_bytes_downloaded: bool,
}
fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn canonical_root(root: &str) -> bool {
    Uuid::parse_str(root).map(|uuid| uuid.to_string() == root).unwrap_or(false)
}
fn expected_path(root: &str, digest: &str) -> String {
    format!("{IMPORT_DIR}/{root}-{digest}.notion.json")
}
fn counts(payload: &Payload, bytes: usize, digest: &str) -> BundleReview {
    let mut object_counts = BTreeMap::new();
    let mut property_types = BTreeMap::new();
    for object in &payload.objects {
        *object_counts.entry(object.kind.clone()).or_insert(0) += 1;
        if object.kind == "page" || object.kind == "data_source" {
            if let Some(props) = object.json.get("properties").and_then(Value::as_object) {
                for prop in props.values() {
                    let kind = prop.get("type").and_then(Value::as_str)
                        .unwrap_or("untyped").to_owned();
                    *property_types.entry(kind).or_insert(0) += 1;
                }
            }
        }
    }
    BundleReview {
        root: payload.root.clone(),
        source_digest: payload.source_digest.clone(),
        payload_blake3: digest.to_owned(),
        relative_path: expected_path(&payload.root, &payload.source_digest),
        bytes,
        object_counts, property_types,
        edge_count: payload.edges.len(),
        workspace_complete: payload.workspace_complete,
        native_database_views_restored: payload.native_database_views_restored,
        media_bytes_downloaded: payload.media_bytes_downloaded,
    }
}

fn encoded(envelope: &Envelope) -> Result<Vec<u8>, String> {
    let raw = serde_json::to_vec(envelope)
        .map_err(|_| "cannot encode independent Notion structured bundle")?;
    if raw.len() > MAX_BUNDLE_BYTES {
        return Err("Notion structured bundle exceeds explicit 64 MiB QA budget".into());
    }
    Ok(raw)
}
fn verify_bundle_bytes(bytes: &[u8]) -> Result<BundleReview, String> {
    if bytes.len() > MAX_BUNDLE_BYTES { return Err("bundle exceeds QA byte limit".into()); }
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| "invalid Notion structured bundle JSON")?;
    let payload = &envelope.payload;
    if payload.format != FORMAT || payload.api_version != API_VERSION
        || !canonical_root(&payload.root) || !valid_digest(&payload.source_digest)
        || !valid_digest(&envelope.payload_blake3)
        || payload.workspace_complete || !payload.root_traversal_complete
        || payload.native_database_views_restored || payload.media_bytes_downloaded
        || payload.objects.is_empty()
    {
        return Err("unsupported or incomplete Notion bundle metadata".into());
    }
    let mut object_ids = BTreeSet::new();
    for object in &payload.objects {
        if object.kind.is_empty() || object.kind.len() > 64
            || object.id.is_empty() || object.id.len() > 512
            || !object_ids.insert((&object.kind, &object.id))
        {
            return Err("invalid or duplicate preserved object identity".into());
        }
    }
    for edge in &payload.edges {
        if edge.from.is_empty() || edge.to.is_empty() || edge.kind.is_empty() {
            return Err("invalid preserved Notion edge".into());
        }
    }
    let canonical = serde_json::to_vec(payload)
        .map_err(|_| "cannot verify Notion bundle payload")?;
    let digest = blake3::hash(&canonical).to_hex().to_string();
    if digest != envelope.payload_blake3 {
        return Err("Notion structured bundle checksum mismatch".into());
    }
    Ok(counts(payload, bytes.len(), &digest))
}

/// Revalidate the full source snapshot checksum before allowing any local
/// write. No Notion credential, network request or Vault access is needed.
pub fn prepare_from_cached(
    ui_state_path: &Path, root: &str, exact_digest: &str,
) -> Result<PreparedBundle, String> {
    if !canonical_root(root) || !valid_digest(exact_digest) {
        return Err("invalid reviewed Notion root or checksum".into());
    }
    let store = SnapshotStore::open_read_only(ui_state_path)?;
    let snapshot = store.load(root)?
        .ok_or("Notion root is absent from the verified API cache")?;
    if snapshot.info.digest != exact_digest {
        return Err("Notion API snapshot changed after user review".into());
    }
    let mut objects: Vec<PreservedObject> = snapshot.objects.into_iter()
        .map(|o: RawObject| PreservedObject { kind:o.kind, id:o.id, json:o.json }).collect();
    objects.sort_by(|a,b| (&a.kind, &a.id).cmp(&(&b.kind, &b.id)));
    let mut edges: Vec<PreservedEdge> = snapshot.links.into_iter()
        .map(|e: Link| PreservedEdge {
            from:e.from, to:e.to, kind:e.kind, property_id:e.property_id,
        }).collect();
    edges.sort_by(|a,b| (&a.from, &a.to, &a.kind, &a.property_id)
        .cmp(&(&b.from, &b.to, &b.kind, &b.property_id)));
    let payload = Payload {
        format:FORMAT.into(), api_version:API_VERSION.into(),
        root:root.into(), source_digest:exact_digest.into(),
        workspace_complete:false, root_traversal_complete:true,
        native_database_views_restored:false, media_bytes_downloaded:false,
        objects, edges,
    };
    let digest = blake3::hash(&serde_json::to_vec(&payload)
        .map_err(|_| "cannot hash Notion bundle payload")?).to_hex().to_string();
    let bytes = encoded(&Envelope {payload, payload_blake3:digest})?;
    let review = verify_bundle_bytes(&bytes)?;
    Ok(PreparedBundle {bytes, review})
}

/// Read an exported independent bundle after API credentials/cache disappear.
/// No cache reads or writes. Exact bytes are bounded before deserialization.
pub fn inspect_existing(path: &Path) -> Result<BundleReview, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| "independent Notion bundle cannot be inspected")?;
    if !meta.is_file() || meta.file_type().is_symlink()
        || meta.len() > MAX_BUNDLE_BYTES as u64
    { return Err("unsafe or oversized independent Notion bundle".into()); }
    let bytes = fs::read(path).map_err(|_| "cannot read independent Notion bundle")?;
    let review = verify_bundle_bytes(&bytes)?;
    if path.file_name().and_then(|s| s.to_str())
        != Path::new(&review.relative_path).file_name().and_then(|s| s.to_str())
    {
        return Err("structured Notion bundle filename disagrees with its source digest".into());
    }
    Ok(review)
}

/// Only after note import has completed and the operator approved this exact
/// root/target. Creates an immutable JSON sidecar; it cannot replace notes or
/// existing structured JSON. This is source preservation, NOT native Bases.
pub fn publish_approved(
    core: &Core, workspace: &Path, bundle: &PreparedBundle, approved: bool,
) -> Result<BundleReceipt, String> {
    if !approved { return Err("independent Notion bundle requires consent".into()); }
    if verify_bundle_bytes(&bundle.bytes)?.payload_blake3 != bundle.review.payload_blake3 {
        return Err("prepared structured bundle changed before File Gate".into());
    }
    let root = fs::canonicalize(workspace)
        .map_err(|_| "cannot resolve selected Akula workspace")?;
    if !root.is_dir() { return Err("selected Akula workspace is not a directory".into()); }
    let known = core.ui_state.workspace_roots()
        .map_err(|_| "cannot inspect registered Akula workspaces")?;
    let matches = known.iter().filter(|path|
        fs::canonicalize(path).ok().as_deref() == Some(root.as_path())
    ).count();
    if matches != 1 { return Err("target is not one uniquely registered workspace".into()); }
    let folder = root.join(IMPORT_DIR);
    let folder_meta = fs::symlink_metadata(&folder)
        .map_err(|_| "Notion-import folder must first be created by approved note import")?;
    if !folder_meta.is_dir() || folder_meta.file_type().is_symlink() {
        return Err("Notion-import folder is not a real directory".into());
    }
    let target = root.join(&bundle.review.relative_path);
    let identical = match fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink()
            && meta.len() <= MAX_BUNDLE_BYTES as u64 =>
        {
            let existing = fs::read(&target)
                .map_err(|_| "cannot verify previous structured bundle")?;
            if existing != bundle.bytes {
                return Err("existing structured Notion bundle differs: no overwrite".into());
            }
            true
        }
        Ok(_) => return Err("unsafe structured bundle target exists".into()),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            gate::create(core, &target, std::str::from_utf8(&bundle.bytes)
                .map_err(|_| "bundle JSON must be UTF-8")?,
                Source::External, Some("Notion typed API snapshot"))
                .map_err(|_| "File Gate refused create-only Notion structured bundle")?;
            false
        }
        Err(_) => return Err("cannot inspect structured bundle target".into()),
    };
    let readback = fs::read(&target)
        .map_err(|_| "cannot verify persisted independent Notion bundle")?;
    if readback != bundle.bytes || verify_bundle_bytes(&readback)?.payload_blake3
        != bundle.review.payload_blake3
    {
        return Err("independent Notion bundle readback mismatch: manual review required".into());
    }
    Ok(BundleReceipt {
        relative_path:bundle.review.relative_path.clone(),
        already_identical: identical,
        verified_bytes:readback.len(),
        payload_blake3:bundle.review.payload_blake3.clone(),
        native_database_views_restored:false,
        media_bytes_downloaded:false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app_core::{Core, CoreEvent},
        notion_api_probe::{projection::prepare_import, publish, SourcePreview},
    };
    use serde_json::json;
    use std::sync::Arc;
    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const C: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    const D: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
    const PROP: &str = "linked%3Arelation";

    fn api() -> SourcePreview {
        SourcePreview {
            format:"synthetic_notion_api", api_version:API_VERSION, root:A.into(),
            workspace_complete:false, root_traversal_complete:true, requests:5,
            objects:vec![
                RawObject {kind:"page".into(), id:A.into(),
                    json:json!({"object":"page","id":A,"properties":{
                        "Name":{"type":"title","title":[{"plain_text":"Root"}]}
                    }})},
                RawObject {kind:"page".into(), id:B.into(),
                    json:json!({"object":"page","id":B,"properties":{
                        "Status":{"type":"select","select":{"id":"todo","name":"Todo"}},
                        "Due":{"type":"date","date":{"start":"2026-10-24"}},
                        "Linked":{"type":"relation","relation":[{"id":A}]}
                    }})},
                RawObject {kind:"data_source".into(),id:C.into(),
                    json:json!({"object":"data_source","id":C,"properties":{
                        "Linked":{"id":PROP,"type":"relation",
                            "relation":{"data_source_id":C}},
                        "Progress":{"id":"p","type":"formula","formula":{"expression":"42"}},
                        "Sum":{"id":"s","type":"rollup","rollup":{"relation_property_id":PROP}}
                    }})},
                RawObject {kind:"view".into(),id:D.into(),
                    json:json!({"object":"view","id":D,"type":"board","filter":{
                        "and":[{"property":"Status","select":{"equals":"Todo"}}]}})},
                RawObject {kind:"markdown".into(),id:A.into(),
                    json:json!({"markdown":"source root body"})},
                RawObject {kind:"markdown".into(),id:B.into(),
                    json:json!({"markdown":"child row body"})},
            ],
            links:vec![Link {from:B.into(),to:A.into(),
                kind:"relation".into(),property_id:Some(PROP.into())}],
            unresolved_target_ids:vec![],warnings:vec![],
        }
    }
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, std::sync::Arc<Core>, String) {
        let temp=tempfile::tempdir().unwrap();
        let app=temp.path().join("app");
        let vault=temp.path().join("vault");
        fs::create_dir(&app).unwrap();
        fs::create_dir(&vault).unwrap();
        let db=app.join("ui-state.sqlite3");
        let digest={
            let mut store=SnapshotStore::open(&db).unwrap();
            store.publish(&api(), 7).unwrap().digest
        };
        let core=Core::open(&app, Arc::new(|_: CoreEvent|{}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
        (temp,db,vault,core,digest)
    }
    #[test]
    fn typed_source_json_and_edges_survive_as_independent_bundle() {
        let (_temp, db, vault, core, digest)=fixture();
        let bundle=prepare_from_cached(&db,A,&digest).unwrap();
        assert_eq!(bundle.review().object_counts.get("data_source"),Some(&1));
        assert_eq!(bundle.review().object_counts.get("view"),Some(&1));
        assert_eq!(bundle.review().object_counts.get("page"),Some(&2));
        assert_eq!(bundle.review().property_types.get("formula"),Some(&1));
        assert_eq!(bundle.review().property_types.get("rollup"),Some(&1));
        assert_eq!(bundle.review().property_types.get("date"),Some(&1));
        let payload: Envelope=serde_json::from_slice(&bundle.bytes).unwrap();
        assert_eq!(payload.payload.edges[0].property_id.as_deref(),Some(PROP));
        let ds=payload.payload.objects.iter().find(|o|o.kind=="data_source").unwrap();
        assert_eq!(ds.json.pointer("/properties/Linked/relation/data_source_id")
            .and_then(Value::as_str),Some(C));
        assert_eq!(payload.payload.objects.iter().find(|o|o.kind=="view").unwrap()
            .json.pointer("/filter/and/0/property").and_then(Value::as_str),Some("Status"));
        assert!(!bundle.review().workspace_complete);
        assert!(!bundle.review().native_database_views_restored);
        assert!(!bundle.review().media_bytes_downloaded);
        assert!(publish_approved(&core,&vault,&bundle,false).is_err());
        assert!(!vault.join(IMPORT_DIR).exists());
        let preview=prepare_import(&api()).unwrap();
        assert!(publish::publish_approved(&core,&vault,&preview,true).unwrap().completed);
        let first=publish_approved(&core,&vault,&bundle,true).unwrap();
        assert!(!first.already_identical);
        let path=vault.join(&first.relative_path);
        let offline=inspect_existing(&path).unwrap();
        assert_eq!(offline.payload_blake3,bundle.review().payload_blake3);
        let second=publish_approved(&core,&vault,&bundle,true).unwrap();
        assert!(second.already_identical);
        assert_eq!(fs::read(&path).unwrap(),bundle.bytes);
        fs::write(&path,b"local edit").unwrap();
        assert!(publish_approved(&core,&vault,&bundle,true).is_err());
        assert_eq!(fs::read(&path).unwrap(),b"local edit");
    }
    #[test]
    fn stale_digest_and_cache_corruption_cannot_create_bundle() {
        let (_temp,db,vault,core,digest)=fixture();
        assert!(prepare_from_cached(&db,A,&"0".repeat(64)).is_err());
        assert!(prepare_from_cached(&db,B,&digest).is_err());
        let conn=rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE notion_cache_objects SET raw_json=?1 WHERE root_id=?2
            AND object_id=?3 AND kind='page'",
            rusqlite::params![r#"{"id":"injected"}"#,A,A]).unwrap();
        assert!(prepare_from_cached(&db,A,&digest).is_err());
        assert!(!vault.join(IMPORT_DIR).exists());
        assert!(core.ui_state.workspace_roots().unwrap().len()>0);
    }
    #[test]
    fn invalid_or_tampered_offline_bundle_fails_without_credentials() {
        let (_temp,db,vault,core,digest)=fixture();
        let bundle=prepare_from_cached(&db,A,&digest).unwrap();
        let preview=prepare_import(&api()).unwrap();
        publish::publish_approved(&core,&vault,&preview,true).unwrap();
        let receipt=publish_approved(&core,&vault,&bundle,true).unwrap();
        let path=vault.join(receipt.relative_path);
        let mut envelope: Envelope=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        envelope.payload.objects[0].json=json!({"injected":"edit"});
        fs::write(&path,serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(inspect_existing(&path).is_err());
    }
}
