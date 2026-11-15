//! Opt-in, read-only Yandex Disk REST metadata client (personal Disk, NOT Yandex S3).
//!
//! This is a metadata discovery boundary, not an implementation of RemoteStore.
//! Yandex's offset-based directory listing is NOT a consistent snapshot. Even
//! the final page cannot authorize deletion, local writes, or baseline promotion.
//! No OAuth flow, credentials persistence, download, upload, rename, or delete
//! is provided by this module. The caller supplies a short-lived OAuth token
//! from an approved OS credential provider and runs this on a worker thread.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

const RESOURCES_ENDPOINT: &str = "https://cloud-api.yandex.net/v1/disk/resources";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_APP_PATH_BYTES: usize = 1024;
const MAX_TOKEN_BYTES: usize = 8192;
pub const MAX_YANDEX_PAGE_SIZE: u16 = 100;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum YandexDiskRestError {
    InvalidAppPath,
    InvalidPage,
    InvalidToken,
    AuthenticationRequired,
    AccessDenied,
    NotFound,
    RateLimited,
    ProviderUnavailable,
    NetworkOrTls,
    ResponseTooLarge,
    InvalidResponse,
    IncompleteListing,
    MissingRootObservation,
}

/// A validated path ONLY within the OAuth app folder: app:/... .
/// No local filesystem path or external URL is accepted here.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct YandexAppPath(String);

impl YandexAppPath {
    pub fn root() -> Self {
        Self("app:/".to_owned())
    }

    pub fn parse(path: &str) -> Result<Self, YandexDiskRestError> {
        if path.len() > MAX_APP_PATH_BYTES {
            return Err(YandexDiskRestError::InvalidAppPath);
        }
        let rest = path
            .strip_prefix("app:/")
            .ok_or(YandexDiskRestError::InvalidAppPath)?;
        if !rest.is_empty() && !rest.split('/').all(valid_segment) {
            return Err(YandexDiskRestError::InvalidAppPath);
        }
        Ok(Self(path.to_owned()))
    }

    pub fn child(&self, name: &str) -> Result<Self, YandexDiskRestError> {
        if !valid_segment(name) {
            return Err(YandexDiskRestError::InvalidAppPath);
        }
        let new_path = if self.0 == "app:/" {
            format!("app:/{name}")
        } else {
            format!("{}/{name}", self.0)
        };
        Self::parse(&new_path)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment
            .chars()
            .any(|c| c == '\\' || c.is_control())
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum YandexDiskEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct YandexDiskEntry {
    pub name: String,
    pub path: YandexAppPath,
    pub kind: YandexDiskEntryKind,
    pub size: Option<u64>,
    /// Provider-supplied hint only. This is NOT an Akula-verified BLAKE3 hash.
    pub remote_md5_hint: Option<String>,
}

/// A point-in-time observation of the app folder's real disk:/ path.
/// The server answers app:/ queries with disk:/ paths. Do not persist or reuse
/// this binding across another account, OAuth identity or connection session.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct YandexAppRootObservation {
    disk_path: String,
}

impl YandexAppRootObservation {
    /// Diagnostic metadata, not a path for API requests.
    pub fn observed_disk_path(&self) -> &str {
        &self.disk_path
    }
}

fn valid_disk_absolute(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("disk:/") else { return false };
    path.len() <= 4096
        && !rest.is_empty()
        && rest.split('/').all(valid_segment)
}

/// A page of observed names, never a verified remote snapshot.
/// The absence of a next page means only that this traversal ended at that time.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct YandexDiskUnverifiedPage {
    /// Provider-reported absolute app-root path, bound to this read session only.
    /// NOT authorization, a stable snapshot, or permission to mutate files.
    pub root_observation: YandexAppRootObservation,
    pub entries: Vec<YandexDiskEntry>,
    pub next_offset: Option<u64>,
    pub total_hint: u64,
}

impl YandexDiskUnverifiedPage {
    pub const fn is_verified_snapshot(&self) -> bool {
        false
    }
}

#[derive(Deserialize)]
struct WireEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<u64>,
    md5: Option<String>,
}

#[derive(Deserialize)]
struct WireEmbedded {
    items: Vec<WireEntry>,
    offset: u64,
    limit: u16,
    total: u64,
}

#[derive(Deserialize)]
struct WireResource {
    #[serde(flatten)]
    entry: WireEntry,
    #[serde(rename = "_embedded")]
    embedded: Option<WireEmbedded>,
}

fn parse_page(
    body: &[u8],
    requested_folder: &YandexAppPath,
    observed_root: Option<&YandexAppRootObservation>,
    requested_offset: u64,
    requested_limit: u16,
) -> Result<YandexDiskUnverifiedPage, YandexDiskRestError> {
    if requested_limit == 0 || requested_limit > MAX_YANDEX_PAGE_SIZE {
        return Err(YandexDiskRestError::InvalidPage);
    }
    if body.len() as u64 > MAX_METADATA_BYTES {
        return Err(YandexDiskRestError::ResponseTooLarge);
    }
    let resource: WireResource =
        serde_json::from_slice(body).map_err(|_| YandexDiskRestError::InvalidResponse)?;

    // The Yandex API takes app:/ paths but replies using absolute disk:/ paths.
    // For the first app:/ root page, the returned disk:/ path becomes a
    // point-in-time observation. Nested paths and subsequent root pages MUST
    // be pinned to that same observation to avoid accepting arbitrary siblings.
    let root_path = if requested_folder.as_str() == "app:/" && requested_offset == 0 {
        if !valid_disk_absolute(&resource.entry.path) {
            return Err(YandexDiskRestError::InvalidResponse);
        }
        let root_path = resource.entry.path.clone();
        if let Some(known) = observed_root {
            if known.disk_path != root_path {
                return Err(YandexDiskRestError::InvalidResponse);
            }
        }
        root_path
    } else {
        observed_root
            .ok_or(YandexDiskRestError::MissingRootObservation)?
            .disk_path
            .clone()
    };
    if !valid_disk_absolute(&root_path) {
        return Err(YandexDiskRestError::InvalidResponse);
    }
    let relative = requested_folder
        .as_str()
        .strip_prefix("app:/")
        .ok_or(YandexDiskRestError::InvalidAppPath)?;
    let expected_folder = if relative.is_empty() {
        root_path.clone()
    } else {
        format!("{root_path}/{relative}")
    };
    if resource.entry.kind != "dir" || resource.entry.path != expected_folder {
        return Err(YandexDiskRestError::InvalidResponse);
    }

    let embedded = resource
        .embedded
        .ok_or(YandexDiskRestError::IncompleteListing)?;
    if embedded.offset != requested_offset
        || embedded.limit == 0
        || embedded.limit > requested_limit
        || embedded.items.len() > requested_limit as usize
    {
        return Err(YandexDiskRestError::InvalidResponse);
    }
    let after = requested_offset
        .checked_add(embedded.items.len() as u64)
        .ok_or(YandexDiskRestError::InvalidResponse)?;
    if after > embedded.total || (after < embedded.total && embedded.items.is_empty()) {
        return Err(YandexDiskRestError::IncompleteListing);
    }

    let prefix = format!("{expected_folder}/");
    let mut seen = BTreeSet::new();
    let mut entries = Vec::with_capacity(embedded.items.len());
    for item in embedded.items {
        if !valid_segment(&item.name) {
            return Err(YandexDiskRestError::InvalidResponse);
        }
        let leaf = item
            .path
            .strip_prefix(&prefix)
            .ok_or(YandexDiskRestError::InvalidResponse)?;
        if leaf.is_empty()
            || leaf.contains('/')
            || leaf != item.name
            || !seen.insert(item.path.clone())
        {
            return Err(YandexDiskRestError::InvalidResponse);
        }
        let kind = match item.kind.as_str() {
            "dir" => YandexDiskEntryKind::Directory,
            "file" if item.size.is_some() => YandexDiskEntryKind::File,
            _ => return Err(YandexDiskRestError::InvalidResponse),
        };
        // Only the derived app:/ alias escapes this parser. The absolute
        // disk:/ path is for validation; NEVER use it in app-scoped API calls.
        entries.push(YandexDiskEntry {
            name: item.name.clone(),
            path: requested_folder.child(&item.name)?,
            kind,
            size: item.size,
            remote_md5_hint: item.md5,
        });
    }

    Ok(YandexDiskUnverifiedPage {
        root_observation: YandexAppRootObservation { disk_path: root_path },
        entries,
        next_offset: (after < embedded.total).then_some(after),
        total_hint: embedded.total,
    })
}

fn classify_http_status(code: u16) -> YandexDiskRestError {
    match code {
        400 => YandexDiskRestError::InvalidResponse,
        401 => YandexDiskRestError::AuthenticationRequired,
        403 => YandexDiskRestError::AccessDenied,
        404 => YandexDiskRestError::NotFound,
        429 => YandexDiskRestError::RateLimited,
        500..=599 => YandexDiskRestError::ProviderUnavailable,
        _ => YandexDiskRestError::ProviderUnavailable,
    }
}

fn classify_http_error(error: ureq::Error) -> YandexDiskRestError {
    match error {
        ureq::Error::Status(code, _) => classify_http_status(code),
        ureq::Error::Transport(_) => YandexDiskRestError::NetworkOrTls,
    }
}

/// Deliberately fixed-origin GET-only API. Never accepts user-controlled hosts,
/// download links, or redirects. No remote writes and no local vault access.
pub struct YandexDiskReadOnlyClient {
    agent: ureq::Agent,
}

impl Default for YandexDiskReadOnlyClient {
    fn default() -> Self {
        Self::new()
    }
}

impl YandexDiskReadOnlyClient {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(4))
            .timeout_read(Duration::from_secs(12))
            .timeout_write(Duration::from_secs(12))
            .redirects(0)
            .build();
        Self { agent }
    }

    /// One bounded directory page. OAuth token must be obtained by the caller
    /// from an approved credential provider and NEVER serialized or logged.
    /// The HTTP library may hold internal copies; zeroing the local header
    /// does not prove all those copies were cleared.
    pub fn list_app_folder_page(
        &self,
        token: &str,
        folder: &YandexAppPath,
        observed_root: Option<&YandexAppRootObservation>,
        offset: u64,
        limit: u16,
    ) -> Result<YandexDiskUnverifiedPage, YandexDiskRestError> {
        if limit == 0 || limit > MAX_YANDEX_PAGE_SIZE {
            return Err(YandexDiskRestError::InvalidPage);
        }
        if token.is_empty()
            || token.len() > MAX_TOKEN_BYTES
            || !token.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(YandexDiskRestError::InvalidToken);
        }

        let authorization = Zeroizing::new(format!("OAuth {token}"));
        let response = self
            .agent
            .get(RESOURCES_ENDPOINT)
            .set("Authorization", authorization.as_str())
            .set("Accept", "application/json")
            .query("path", folder.as_str())
            .query("limit", &limit.to_string())
            .query("offset", &offset.to_string())
            .call()
            .map_err(classify_http_error)?;
        let mut bytes = Vec::new();
        let mut reader = response.into_reader().take(MAX_METADATA_BYTES + 1);
        reader
            .read_to_end(&mut bytes)
            .map_err(|_| YandexDiskRestError::NetworkOrTls)?;
        parse_page(&bytes, folder, observed_root, offset, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REAL_ROOT: &str = "disk:/Приложения/Akula";

    fn folder() -> YandexAppPath {
        YandexAppPath::parse("app:/notes").unwrap()
    }

    fn wire_response(
        name: &str,
        disk_path: &str,
        offset: u64,
        total: u64,
        items: serde_json::Value,
    ) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "name": name,
            "path": disk_path,
            "type": "dir",
            "_embedded": {
                "items": items,
                "offset": offset,
                "limit": 2,
                "total": total
            }
        }))
        .unwrap()
    }

    fn response(offset: u64, total: u64, items: serde_json::Value) -> Vec<u8> {
        wire_response("notes", &format!("{REAL_ROOT}/notes"), offset, total, items)
    }

    fn root_observation() -> YandexAppRootObservation {
        let root = wire_response("Akula", REAL_ROOT, 0, 0, json!([]));
        parse_page(&root, &YandexAppPath::root(), None, 0, 2)
            .unwrap()
            .root_observation
    }

    #[test]
    fn app_folder_paths_reject_escape_and_control_characters() {
        let root = YandexAppPath::root();
        assert_eq!(root.child("chapter.md").unwrap().as_str(), "app:/chapter.md");
        assert_eq!(folder().child("Глава 1.md").unwrap().as_str(), "app:/notes/Глава 1.md");
        for path in [
            "disk:/notes",
            "app:/../notes",
            "app:/notes//bad",
            "app:/notes/.",
            "app:/notes\\bad",
            "app:/notes/\n",
        ] {
            assert_eq!(
                YandexAppPath::parse(path),
                Err(YandexDiskRestError::InvalidAppPath)
            );
        }
        assert_eq!(root.child("../escape"), Err(YandexDiskRestError::InvalidAppPath));
    }

    #[test]
    fn official_app_folder_response_is_disk_absolute_but_exposed_as_app_alias() {
        let root_body = wire_response("Akula", REAL_ROOT, 0, 1, json!([
            {"name": "notes", "path": "disk:/Приложения/Akula/notes", "type": "dir"}
        ]));
        let root_page = parse_page(&root_body, &YandexAppPath::root(), None, 0, 2).unwrap();
        assert_eq!(root_page.root_observation.observed_disk_path(), REAL_ROOT);
        assert_eq!(root_page.entries[0].path.as_str(), "app:/notes");
        assert!(!root_page.is_verified_snapshot());
        assert!(parse_page(&root_body, &YandexAppPath::root(), Some(&root_observation()), 0, 2).is_ok());
    }

    #[test]
    fn nested_page_requires_prior_app_root_and_canonical_absolute_parent() {
        let nested = response(0, 1, json!([
            {"name": "Глава.md", "path": "disk:/Приложения/Akula/notes/Глава.md", "type": "file", "size": 12}
        ]));
        assert_eq!(
            parse_page(&nested, &folder(), None, 0, 2),
            Err(YandexDiskRestError::MissingRootObservation)
        );
        let root = root_observation();
        let page = parse_page(&nested, &folder(), Some(&root), 0, 2).unwrap();
        assert_eq!(page.entries[0].path.as_str(), "app:/notes/Глава.md");
        assert_eq!(page.root_observation, root);

        let other_root = YandexAppRootObservation { disk_path: "disk:/Apps/OtherApp".to_string() };
        assert!(parse_page(&nested, &folder(), Some(&other_root), 0, 2).is_err());
        assert!(parse_page(&nested, &folder(), Some(&root), 1, 2).is_err());
    }

    #[test]
    fn pages_are_bounded_and_always_unverified() {
        let root = root_observation();
        let page1 = response(0, 3, json!([
            {"name":"a.md","path":"disk:/Приложения/Akula/notes/a.md","type":"file","size":7,"md5":"abc"},
            {"name":"sub","path":"disk:/Приложения/Akula/notes/sub","type":"dir"}
        ]));
        let parsed = parse_page(&page1, &folder(), Some(&root), 0, 2).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].size, Some(7));
        assert_eq!(parsed.next_offset, Some(2));
        assert!(!parsed.is_verified_snapshot());

        let page2 = response(2, 3, json!([
            {"name":"b.md","path":"disk:/Приложения/Akula/notes/b.md","type":"file","size":0}
        ]));
        let parsed = parse_page(&page2, &folder(), Some(&root), 2, 2).unwrap();
        assert_eq!(parsed.next_offset, None);
        assert!(!parsed.is_verified_snapshot());
    }

    #[test]
    fn offset_drift_and_truncated_pages_fail_closed() {
        let root = root_observation();
        let empty = response(0, 2, json!([]));
        assert_eq!(
            parse_page(&empty, &folder(), Some(&root), 0, 2),
            Err(YandexDiskRestError::IncompleteListing)
        );
        let stale = response(0, 1, json!([
            {"name":"a.md","path":"disk:/Приложения/Akula/notes/a.md","type":"file","size":1}
        ]));
        assert_eq!(
            parse_page(&stale, &folder(), Some(&root), 2, 2),
            Err(YandexDiskRestError::InvalidResponse)
        );
        assert_eq!(
            parse_page(&stale, &folder(), Some(&root), 0, 0),
            Err(YandexDiskRestError::InvalidPage)
        );
    }

    #[test]
    fn foreign_nested_duplicate_or_untyped_items_are_rejected() {
        let root = root_observation();
        let cases = [
            json!([{"name":"foreign.md","path":"disk:/Other/foreign.md","type":"file","size":1}]),
            json!([{"name":"deep.md","path":"disk:/Приложения/Akula/notes/sub/deep.md","type":"file","size":1}]),
            json!([
                {"name":"a.md","path":"disk:/Приложения/Akula/notes/a.md","type":"file","size":1},
                {"name":"a.md","path":"disk:/Приложения/Akula/notes/a.md","type":"file","size":1}
            ]),
            json!([{"name":"a.md","path":"disk:/Приложения/Akula/notes/a.md","type":"file"}]),
            json!([{"name":"a.md","path":"disk:/Приложения/Akula/notes/a.md","type":"link","size":1}]),
            json!([{"name":"escape","path":"disk:/Приложения/Akula/notes/..","type":"dir"}]),
        ];
        for items in cases {
            let count = items.as_array().unwrap().len() as u64;
            assert!(
                parse_page(&response(0, count, items), &folder(), Some(&root), 0, 2).is_err()
            );
        }
    }

    #[test]
    fn invalid_app_root_and_root_drift_do_not_grant_listing() {
        for path in ["app:/", "disk:/", "disk:/Apps/../Other", "disk:/Apps//Child", "disk:/Apps\\Foo", "disk:/Apps/\nFoo"] {
            let b = wire_response("Akula", path, 0, 0, json!([]));
            assert!(parse_page(&b, &YandexAppPath::root(), None, 0, 2).is_err());
        }
        let root = root_observation();
        let changed = wire_response("Other", "disk:/Apps/Other", 1, 1, json!([]));
        assert!(parse_page(&changed, &YandexAppPath::root(), Some(&root), 1, 2).is_err());
    }

    #[test]
    fn failed_authorization_or_server_status_never_becomes_empty_listing() {
        assert_eq!(classify_http_status(401), YandexDiskRestError::AuthenticationRequired);
        assert_eq!(classify_http_status(403), YandexDiskRestError::AccessDenied);
        assert_eq!(classify_http_status(404), YandexDiskRestError::NotFound);
        assert_eq!(classify_http_status(429), YandexDiskRestError::RateLimited);
        assert_eq!(classify_http_status(503), YandexDiskRestError::ProviderUnavailable);
        assert!(serde_json::from_slice::<WireResource>(b"[]").is_err());
    }

    #[test]
    fn metadata_size_limit_denies_oversized_payloads() {
        let payload = vec![b'x'; (MAX_METADATA_BYTES + 1) as usize];
        assert_eq!(
            parse_page(&payload, &folder(), Some(&root_observation()), 0, 2),
            Err(YandexDiskRestError::ResponseTooLarge)
        );
    }
}
