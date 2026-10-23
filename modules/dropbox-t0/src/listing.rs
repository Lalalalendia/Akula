//! Bounded, scoped Dropbox list_folder and cursor parsing.
//! A Dropbox deleted record is transport metadata, not an Akula DAG tombstone.
use crate::{parse_metadata, DropboxFileRef, Error, ObjectKey, Scope};
use serde::Deserialize;
use serde_json::{json, Value};

pub const LIST_ROUTE: &str = "/2/files/list_folder";
pub const LATEST_CURSOR_ROUTE: &str = "/2/files/list_folder/get_latest_cursor";

/// An inert, read-only Dropbox listing request for an existing scoped Vault.
/// Unlike the upstream Aquilum connector this never creates remote folders.
pub fn prepare_list(scope: &Scope) -> Result<String, Error> {
    let path = format!("/akula-sync/v1/vaults/{}", scope.vault_id());
    if path.is_empty() { return Err(Error::InvalidScope); }
    Ok(json!({"path": path, "recursive": true, "include_deleted": false}).to_string())
}

/// A validated, read-only Dropbox delta page. Only entries inside the
/// configured Vault are returned; deleted entries are explicit tombstones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropboxDeltaPage {
    pub entries: Vec<DropboxDeltaEntry>,
    pub cursor: String,
    pub has_more: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropboxDeltaEntry {
    File(DropboxFileRef),
    Deleted(ObjectKey),
}

/// Parse list_folder and list_folder/continue using the same pinned scope.
/// Never infer an upload/delete action from this untrusted metadata.
pub fn parse_delta_page(scope: &Scope, bytes: &[u8]) -> Result<DropboxDeltaPage, Error> {
    #[derive(Deserialize)]
    struct WirePage {
        entries: Vec<Value>,
        cursor: String,
        has_more: bool,
    }
    if bytes.len() > 8 * 1024 * 1024 { return Err(Error::InvalidMetadata); }
    let page: WirePage = serde_json::from_slice(bytes).map_err(|_| Error::InvalidMetadata)?;
    if page.entries.len() > 10_000 || page.cursor.is_empty()
        || page.cursor.len() > 4096 || page.cursor.chars().any(char::is_control)
    { return Err(Error::InvalidCursor); }
    let prefix = format!("/akula-sync/v1/vaults/{}/", scope.vault_id());
    let mut entries = Vec::with_capacity(page.entries.len());
    let mut seen = std::collections::HashSet::new();
    for raw in page.entries {
        let tag = raw.get(".tag").and_then(Value::as_str).ok_or(Error::InvalidMetadata)?;
        let path = raw.get("path_lower").and_then(Value::as_str)
            .ok_or(Error::InvalidMetadata)?;
        // Dropbox paths are case-insensitive. Require the canonical Vault
        // prefix and refuse unknown or ambiguous logical object names.
        if !path.to_ascii_lowercase().starts_with(&prefix) {
            return Err(Error::InvalidMetadata);
        }
        let suffix = &path[prefix.len()..];
        // Recursive Dropbox listings include the four structural folders.
        // They are not sync objects or deletions; skip only the exact names.
        // A deleted folder, foreign folder or unknown directory fails closed.
        if tag == "folder" {
            if !matches!(suffix, "refs" | "blobs" | "manifests" | "revisions") {
                return Err(Error::InvalidMetadata);
            }
            continue;
        }
        let key = ObjectKey::parse(suffix).map_err(|_| Error::InvalidMetadata)?;
        if !seen.insert(key.as_str().to_owned()) { return Err(Error::InvalidMetadata); }
        let entry = match tag {
            "file" => {
                let wire = serde_json::to_vec(&raw).map_err(|_| Error::InvalidMetadata)?;
                DropboxDeltaEntry::File(parse_metadata(scope, &key, &wire)?)
            }
            "deleted" => DropboxDeltaEntry::Deleted(key),
            _ => return Err(Error::InvalidMetadata),
        };
        entries.push(entry);
    }
    Ok(DropboxDeltaPage { entries, cursor: page.cursor, has_more: page.has_more })
}

#[cfg(test)]
mod delta_page_tests {
    use super::*;
    fn scope() -> Scope {
        Scope::new("dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "home").unwrap()
    }
    fn path() -> String {
        scope().object_path(&ObjectKey::head()).to_ascii_lowercase()
    }
    #[test]
    fn files_and_tombstones_are_distinct_and_read_only() {
        let p = path();
        let page = serde_json::json!({
            "cursor":"cursor-1","has_more":false,"entries":[
                {".tag":"file","id":"id:abc","rev":"a1","size":3,
                    "path_lower":p,"is_downloadable":true}
            ]
        });
        let result = parse_delta_page(&scope(), &serde_json::to_vec(&page).unwrap()).unwrap();
        assert_eq!(result.entries.len(),1);
        assert!(matches!(&result.entries[0],DropboxDeltaEntry::File(f) if f.logical_key.is_head()));
        let tombstone = serde_json::json!({"cursor":"cursor-2","has_more":false,
            "entries":[{".tag":"deleted","path_lower":path()}]});
        assert!(matches!(parse_delta_page(&scope(), &serde_json::to_vec(&tombstone).unwrap())
            .unwrap().entries[0], DropboxDeltaEntry::Deleted(_)));
    }
    #[test]
    fn known_structural_folders_are_skipped_but_folder_deletions_hold() {
        let prefix = format!("/akula-sync/v1/vaults/{}", scope().vault_id());
        let mut entries = vec![];
        for name in ["refs", "blobs", "manifests", "revisions"] {
            entries.push(serde_json::json!({
                ".tag":"folder","path_lower":format!("{prefix}/{name}")
            }));
        }
        entries.push(serde_json::json!({
            ".tag":"file","path_lower":path(),
            "id":"id:abc","rev":"a1","size":3,"is_downloadable":true
        }));
        let page = serde_json::json!({
            "cursor":"cursor","has_more":false,"entries":entries
        });
        let parsed = parse_delta_page(&scope(), &serde_json::to_vec(&page).unwrap()).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        let root_deleted = serde_json::json!({
            "cursor":"cursor","has_more":false,"entries":[
                {".tag":"deleted","path_lower":format!("{prefix}/refs")}
            ]
        });
        assert_eq!(parse_delta_page(&scope(), &serde_json::to_vec(&root_deleted).unwrap()),
            Err(Error::InvalidMetadata));
        let foreign_folder = serde_json::json!({
            "cursor":"cursor","has_more":false,"entries":[
                {".tag":"folder","path_lower":format!("{prefix}/unexpected")}
            ]
        });
        assert_eq!(parse_delta_page(&scope(), &serde_json::to_vec(&foreign_folder).unwrap()),
            Err(Error::InvalidMetadata));
    }
    #[test]
    fn foreign_paths_duplicate_keys_and_unknown_tags_are_rejected() {
        let entry = serde_json::json!({".tag":"deleted","path_lower":path()});
        for entries in [
            vec![entry.clone(),entry.clone()],
            vec![serde_json::json!({".tag":"deleted","path_lower":"/other/refs/current.json"})],
            vec![serde_json::json!({".tag":"folder","path_lower":path()})],
        ] {
            let page = serde_json::json!({"cursor":"cursor","has_more":false,"entries":entries});
            assert!(parse_delta_page(&scope(), &serde_json::to_vec(&page).unwrap()).is_err());
        }
    }
}

/// Dropbox cursor pages are untrusted: only exact, non-empty cursors are accepted.
pub fn parse_list_cursor(bytes: &[u8]) -> Result<(String, bool), Error> {
    #[derive(Deserialize)]
    struct PageCursor { cursor: String, has_more: bool }
    let page: PageCursor = serde_json::from_slice(bytes)
        .map_err(|_| Error::InvalidCursor)?;
    if page.cursor.is_empty() || page.cursor.len() > 4096
        || page.cursor.chars().any(char::is_control) {
        return Err(Error::InvalidCursor);
    }
    Ok((page.cursor, page.has_more))
}

#[cfg(test)]
mod listing_tests {
    use super::*;
    #[test]
    fn listing_never_requests_folder_creation() {
        let scope = Scope::new("dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "test").unwrap();
        let request = prepare_list(&scope).unwrap();
        assert!(request.contains("12345678-1234-1234-1234-123456789abc"));
        assert_eq!(serde_json::from_str::<serde_json::Value>(&request).unwrap()["recursive"], serde_json::json!(true));
        assert!(!request.contains("create_folder"));
        assert_eq!(LIST_ROUTE, "/2/files/list_folder");
    }
    #[test]
    fn invalid_cursor_fails_closed() {
        assert_eq!(parse_list_cursor(br#"{"cursor":"","has_more":true}"#),
            Err(Error::InvalidCursor));
        assert_eq!(parse_list_cursor(br#"{"cursor":"abc","has_more":false}"#),
            Ok(("abc".to_owned(), false)));
    }
}
