#![forbid(unsafe_code)]
//! Isolated Dropbox contracts, no local file mutation or sync authority.
pub mod publication;
pub mod read_verify;
pub mod read_http;

use serde::Deserialize;
use serde_json::{json, Value};

pub const UPLOAD_ROUTE: &str = "/2/files/upload";
pub const METADATA_ROUTE: &str = "/2/files/get_metadata";
pub const CONTINUE_ROUTE: &str = "/2/files/list_folder/continue";

mod listing;
pub use listing::{prepare_list, parse_delta_page, parse_list_cursor,
    DropboxDeltaEntry, DropboxDeltaPage, LIST_ROUTE, LATEST_CURSOR_ROUTE};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidScope,
    InvalidObjectKey,
    InvalidRevision,
    InvalidMetadata,
    AuthRequired,
    PermissionDenied,
    RateLimited(Option<u64>),
    PreconditionFailed,
    InvalidCursor,
    CommitUncertain,
    Unavailable,
    UnexpectedResponse,
    Unsupported,
}

fn uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36 && bytes.iter().enumerate().all(|(index, byte)| {
        if [8, 13, 18, 23].contains(&index) {
            *byte == b'-'
        } else {
            byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
        }
    })
}

fn lowercase_hash(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn safe_label(label: &str, max: usize) -> bool {
    !label.is_empty() && label.len() <= max
        && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    account_id: String,
    root_namespace_id: String,
    vault_id: String,
    profile_id: String,
}

impl Scope {
    pub fn new(account_id: &str, root_namespace_id: &str, vault_id: &str, profile_id: &str)
        -> Result<Self, Error>
    {
        let account = account_id.strip_prefix("dbid:").ok_or(Error::InvalidScope)?;
        if !safe_label(account, 128) || root_namespace_id.is_empty()
            || root_namespace_id.len() > 32
            || !root_namespace_id.bytes().all(|c| c.is_ascii_digit())
            || !uuid(vault_id) || !safe_label(profile_id, 64)
        {
            return Err(Error::InvalidScope);
        }
        Ok(Self {
            account_id: account_id.to_owned(),
            root_namespace_id: root_namespace_id.to_owned(),
            vault_id: vault_id.to_owned(),
            profile_id: profile_id.to_owned(),
        })
    }

    pub fn object_path(&self, key: &ObjectKey) -> String {
        format!("/akula-sync/v1/vaults/{}/{}", self.vault_id, key.as_str())
    }

    pub fn account_id(&self) -> &str { &self.account_id }
    pub fn root_namespace_id(&self) -> &str { &self.root_namespace_id }
    pub fn vault_id(&self) -> &str { &self.vault_id }
    pub fn profile_id(&self) -> &str { &self.profile_id }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn parse(raw: &str) -> Result<Self, Error> {
        if raw == "refs/current.json" {
            return Ok(Self(raw.to_owned()));
        }
        let (kind, value) = raw.split_once('/').ok_or(Error::InvalidObjectKey)?;
        let valid = match kind {
            "blobs" => lowercase_hash(value),
            "manifests" | "revisions" => value.strip_suffix(".json").is_some_and(uuid),
            _ => false,
        };
        if !valid { return Err(Error::InvalidObjectKey); }
        Ok(Self(raw.to_owned()))
    }

    pub fn head() -> Self { Self("refs/current.json".to_owned()) }
    pub fn as_str(&self) -> &str { &self.0 }
    pub fn is_head(&self) -> bool { self.0 == "refs/current.json" }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropboxRevision(String);

impl DropboxRevision {
    pub fn parse(value: &str) -> Result<Self, Error> {
        if !safe_label(value, 128) { return Err(Error::InvalidRevision); }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropboxContentHash(String);

impl DropboxContentHash {
    pub fn parse(value: &str) -> Result<Self, Error> {
        if !lowercase_hash(value) { return Err(Error::InvalidMetadata); }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteGuard {
    Absent,
    ExactRevision(DropboxRevision),
}

/// Describes a request. Deliberately does NOT hold an auth token or send bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    pub scope: Scope,
    pub logical_key: ObjectKey,
    pub route: &'static str,
    pub api_arg: String,
}

pub fn prepare_upload(
    scope: &Scope, key: &ObjectKey, guard: WriteGuard,
) -> Result<PreparedRequest, Error> {
    let mode = match guard {
        WriteGuard::Absent => json!("add"),
        WriteGuard::ExactRevision(revision) if key.is_head() =>
            json!({".tag": "update", "update": revision.as_str()}),
        WriteGuard::ExactRevision(_) => return Err(Error::Unsupported),
    };
    let api_arg = json!({
        "path": scope.object_path(key),
        "mode": mode,
        "autorename": false,
        "strict_conflict": true,
        "mute": true
    }).to_string();
    debug_assert!(api_arg.is_ascii());
    Ok(PreparedRequest {
        scope: scope.clone(),
        logical_key: key.clone(),
        route: UPLOAD_ROUTE,
        api_arg,
    })
}

pub fn prepare_metadata(scope: &Scope, key: &ObjectKey) -> PreparedRequest {
    PreparedRequest {
        scope: scope.clone(),
        logical_key: key.clone(),
        route: METADATA_ROUTE,
        api_arg: json!({"path": scope.object_path(key)}).to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropboxFileRef {
    /// The requested logical object key, never the Dropbox-native file id.
    pub logical_key: ObjectKey,
    pub native_file_id: String,
    pub revision: DropboxRevision,
    pub byte_length: u64,
    /// Dropbox SHA-256 tree hash; never assign to Akula RemoteObject.content_hash.
    pub dropbox_hash: Option<DropboxContentHash>,
    pub downloadable: bool,
}

#[derive(Deserialize)]
struct FileMetadataWire {
    #[serde(rename = ".tag")]
    tag: Option<String>,
    id: String,
    rev: String,
    size: u64,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    path_display: Option<String>,
    #[serde(default)]
    path_lower: Option<String>,
    #[serde(default)]
    is_downloadable: Option<bool>,
}

pub fn parse_metadata(
    scope: &Scope, key: &ObjectKey, bytes: &[u8],
) -> Result<DropboxFileRef, Error> {
    let wire: FileMetadataWire =
        serde_json::from_slice(bytes).map_err(|_| Error::InvalidMetadata)?;
    if wire.tag.as_deref().is_some_and(|tag| tag != "file")
        || !wire.id.starts_with("id:")
        || !safe_label(&wire.id[3..], 256)
    {
        return Err(Error::InvalidMetadata);
    }
    let expected = scope.object_path(key);
    for path in [wire.path_display.as_deref(), wire.path_lower.as_deref()].into_iter().flatten() {
        if !path.eq_ignore_ascii_case(&expected) {
            return Err(Error::InvalidMetadata);
        }
    }
    let revision = DropboxRevision::parse(&wire.rev)?;
    let hash = wire.content_hash
        .map(|value| DropboxContentHash::parse(&value))
        .transpose()?;
    Ok(DropboxFileRef {
        logical_key: key.clone(),
        native_file_id: wire.id,
        revision,
        byte_length: wire.size,
        dropbox_hash: hash,
        downloadable: wire.is_downloadable == Some(true),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Read,
    Upload,
    Continue,
}

/// Classify structured Dropbox errors, NOT error_summary substring guesses.
pub fn classify_http(
    operation: Operation, status: u16, body: &[u8], retry_after: Option<&str>,
) -> Result<(), Error> {
    match status {
        200..=299 => Ok(()),
        401 => Err(Error::AuthRequired),
        403 => Err(Error::PermissionDenied),
        429 => Err(Error::RateLimited(
            retry_after.and_then(|x| x.parse::<u64>().ok()).map(|x| x.min(3600)),
        )),
        500..=599 if operation == Operation::Upload => Err(Error::CommitUncertain),
        500..=599 => Err(Error::Unavailable),
        409 => {
            let data: Value = serde_json::from_slice(body)
                .map_err(|_| Error::UnexpectedResponse)?;
            let tag = data.pointer("/error/.tag").and_then(Value::as_str);
            let nested = data.pointer("/error/path/.tag").and_then(Value::as_str);
            match (operation, tag, nested) {
                (Operation::Upload, Some("path"), Some("conflict")) =>
                    Err(Error::PreconditionFailed),
                (Operation::Continue, Some("reset"), _) => Err(Error::InvalidCursor),
                _ => Err(Error::UnexpectedResponse),
            }
        }
        _ => Err(Error::UnexpectedResponse),
    }
}

/// A write may have completed even when its HTTP response never arrived.
pub fn classify_network_loss(operation: Operation) -> Error {
    if operation == Operation::Upload { Error::CommitUncertain }
    else { Error::Unavailable }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> Scope {
        Scope::new("dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "home").unwrap()
    }

    #[test]
    fn scope_and_key_cannot_escape_vault() {
        assert!(Scope::new("other", "123", scope().vault_id(), "home").is_err());
        assert!(Scope::new("dbid:alice", "../123", scope().vault_id(), "home").is_err());
        assert!(Scope::new("dbid:alice", "123", "invalid", "home").is_err());
        for key in ["../head", "/refs/current.json", "blobs/../foo",
            "refs/other.json", "manifests/hello.json", "blobs/FFFF", "notes/a.md",
            "refs/current.json/extra", "blobs/aa\\bb"] {
            assert!(ObjectKey::parse(key).is_err(), "{key}");
        }
        assert_eq!(scope().object_path(&ObjectKey::head()),
            "/akula-sync/v1/vaults/12345678-1234-1234-1234-123456789abc/refs/current.json");
    }

    #[test]
    fn no_unconditional_replacement_or_delete() {
        let head = ObjectKey::head();
        let revision = DropboxRevision::parse("R0").unwrap();
        let upload = prepare_upload(&scope(), &head,
            WriteGuard::ExactRevision(revision)).unwrap();
        let value: Value = serde_json::from_str(&upload.api_arg).unwrap();
        assert_eq!(value["mode"][".tag"], "update");
        assert_eq!(value["mode"]["update"], "R0");
        assert_eq!(value["autorename"], false);
        assert_eq!(value["strict_conflict"], true);
        assert_eq!(upload.route, UPLOAD_ROUTE);
        let blob = ObjectKey::parse(&format!("blobs/{}", "a".repeat(64))).unwrap();
        assert_eq!(prepare_upload(&scope(), &blob,
            WriteGuard::ExactRevision(DropboxRevision::parse("R0").unwrap())),
            Err(Error::Unsupported));
        let add = prepare_upload(&scope(), &blob, WriteGuard::Absent).unwrap();
        let v: Value = serde_json::from_str(&add.api_arg).unwrap();
        assert_eq!(v["mode"], "add");
    }

    #[test]
    fn absent_nullable_metadata_and_distinct_hash_types() {
        let key = ObjectKey::head();
        let data = br#"{"id":"id:abc","rev":"R1","size":3,"content_hash":null,
            "path_display":null,"is_downloadable":true}"#;
        let file = parse_metadata(&scope(), &key, data).unwrap();
        assert_eq!(file.logical_key, key);
        assert_eq!(file.native_file_id, "id:abc");
        assert_eq!(file.dropbox_hash, None);
        assert!(file.downloadable);
        assert!(parse_metadata(&scope(), &ObjectKey::head(),
            br#"{"id":"id:abc","rev":"R1","size":3,"path_lower":"/other"}"#).is_err());
        let hash = "a".repeat(64);
        let data = format!(r#"{{"id":"id:abc","rev":"R1","size":3,
            "content_hash":"{hash}"}}"#);
        assert_eq!(parse_metadata(&scope(), &ObjectKey::head(), data.as_bytes())
            .unwrap().dropbox_hash.unwrap().as_str(), hash);
    }

    #[test]
    fn structured_conflicts_and_network_ambiguity() {
        let conflict = br#"{"error_summary":"reset/not_found/misleading",
            "error":{".tag":"path","path":{".tag":"conflict"}}}"#;
        assert_eq!(classify_http(Operation::Upload, 409, conflict, None),
            Err(Error::PreconditionFailed));
        assert_eq!(classify_http(Operation::Continue, 409,
            br#"{"error":{".tag":"reset"}}"#, None), Err(Error::InvalidCursor));
        assert_eq!(classify_http(Operation::Upload, 409,
            br#"{"error_summary":"conflict","error":{".tag":"path"}}"#, None),
            Err(Error::UnexpectedResponse));
        assert_eq!(classify_http(Operation::Upload, 503, b"", None),
            Err(Error::CommitUncertain));
        assert_eq!(classify_network_loss(Operation::Upload), Error::CommitUncertain);
        assert_eq!(classify_http(Operation::Read, 429, b"", Some("120")),
            Err(Error::RateLimited(Some(120))));
        assert_eq!(classify_http(Operation::Read, 401, b"", None),
            Err(Error::AuthRequired));
        assert_eq!(classify_http(Operation::Read, 403, b"", None),
            Err(Error::PermissionDenied));
    }
}
