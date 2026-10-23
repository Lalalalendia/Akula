//! Opt-in, read-only Dropbox HTTPS client for the standalone T1 laboratory.
//!
//! This does not implement OAuth, refresh tokens, filesystem writes, sync,
//! background tasks or a provider adapter. Token must come from a separately
//! authorized secret source. Hosts are hard-coded in production; redirects
//! are disabled so Authorization is never forwarded to a third-party URL.

use crate::{
    classify_http, parse_metadata, DropboxFileRef, ObjectKey, Operation, Scope,
};
use crate::read_verify::{
    pinned_download_arg, verify_download, TrustedBlake3, VerifiedRead, VerifyError,
    MAX_METADATA_BYTES, MAX_READ_BYTES,
};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};
use zeroize::Zeroize;

const API: &str = "https://api.dropboxapi.com";
const CONTENT: &str = "https://content.dropboxapi.com";

pub struct AccessToken(String);

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessToken(REDACTED)")
    }
}

impl Drop for AccessToken {
    fn drop(&mut self) { self.0.zeroize(); }
}

impl AccessToken {
    pub fn new(raw: String) -> Result<Self, ReadError> {
        if raw.is_empty() || raw.len() > 4096 || !raw.bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
        {
            return Err(ReadError::InvalidToken);
        }
        Ok(Self(raw))
    }
    fn bearer(&self) -> String { format!("Bearer {}", self.0) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    InvalidToken,
    AccountMismatch,
    InvalidAccountMetadata,
    NetworkUnavailable,
    Protocol(crate::Error),
    Metadata(crate::Error),
    NoDownloadPermission,
    ResponseTooLarge,
    MissingResultHeader,
    Verify(VerifyError),
}

impl From<VerifyError> for ReadError {
    fn from(err: VerifyError) -> Self { Self::Verify(err) }
}

struct Endpoints { api: String, content: String }

/// Only [Self::connect] is publicly accessible; tests use loopback endpoints.
pub struct DropboxReadOnly {
    agent: ureq::Agent,
    token: AccessToken,
    scope: Scope,
    endpoints: Endpoints,
}

impl DropboxReadOnly {
    pub fn connect(scope: Scope, token: AccessToken) -> Result<Self, ReadError> {
        Self::connect_at(scope, token, Endpoints {
            api: API.to_owned(), content: CONTENT.to_owned(),
        })
    }

    fn connect_at(scope: Scope, token: AccessToken, endpoints: Endpoints)
        -> Result<Self, ReadError>
    {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(20))
            .timeout_write(Duration::from_secs(20))
            .build();
        let client = Self { agent, token, scope, endpoints };
        client.verify_account()?;
        Ok(client)
    }

    /// Refuses to read if the token belongs to a different Dropbox account
    /// or to a different root namespace. This requires account_info.read.
    fn verify_account(&self) -> Result<(), ReadError> {
        let url = format!("{}/2/users/get_current_account", self.endpoints.api);
        let response = self.agent.post(&url)
            .set("Authorization", &self.token.bearer())
            .call().map_err(Self::from_ureq)?;
        let bytes = bounded(response, MAX_METADATA_BYTES)?;
        let root: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ReadError::InvalidAccountMetadata)?;
        let account = root.get("account_id").and_then(Value::as_str);
        let namespace = root.pointer("/root_info/root_namespace_id").and_then(Value::as_str);
        if account.is_none() || namespace.is_none() {
            return Err(ReadError::InvalidAccountMetadata);
        }
        if account != Some(self.scope.account_id())
            || namespace != Some(self.scope.root_namespace_id())
        {
            return Err(ReadError::AccountMismatch);
        }
        Ok(())
    }

    pub fn metadata(&self, key: &ObjectKey) -> Result<DropboxFileRef, ReadError> {
        let url = format!("{}/2/files/get_metadata", self.endpoints.api);
        let json = json!({
            "path": self.scope.object_path(key),
            "include_deleted": false
        }).to_string();
        let response = self.agent.post(&url)
            .set("Authorization", &self.token.bearer())
            .set("Content-Type", "application/json")
            .send_string(&json).map_err(Self::from_ureq)?;
        let bytes = bounded(response, MAX_METADATA_BYTES)?;
        let file = parse_metadata(&self.scope, key, &bytes)
            .map_err(ReadError::Metadata)?;
        if !file.downloadable { return Err(ReadError::NoDownloadPermission); }
        Ok(file)
    }

    /// Fetch an existing Vault listing or cursor delta without creating
    /// remote folders or applying changes to local files.
    pub fn list_delta(&self, cursor: Option<&str>) -> Result<crate::DropboxDeltaPage, ReadError> {
        let (route, body) = match cursor {
            Some(value) => {
                if value.is_empty() || value.len() > 4096
                    || value.chars().any(char::is_control)
                { return Err(ReadError::Protocol(crate::Error::InvalidCursor)); }
                (crate::CONTINUE_ROUTE, json!({"cursor":value}).to_string())
            }
            None => (crate::LIST_ROUTE, crate::prepare_list(&self.scope)
                .map_err(ReadError::Protocol)?),
        };
        let url = format!("{}{}", self.endpoints.api, route);
        let response = self.agent.post(&url)
            .set("Authorization", &self.token.bearer())
            .set("Content-Type", "application/json")
            .send_string(&body).map_err(|error| {
                match error {
                    ureq::Error::Status(409, response) if cursor.is_some() => {
                        let bytes = bounded(response, 4096).unwrap_or_default();
                        let err = classify_http(Operation::Continue, 409, &bytes, None)
                            .err().unwrap_or(crate::Error::UnexpectedResponse);
                        ReadError::Protocol(err)
                    }
                    other => Self::from_ureq(other),
                }
            })?;
        let bytes = bounded(response, 8 * 1024 * 1024)?;
        crate::parse_delta_page(&self.scope, &bytes).map_err(ReadError::Metadata)
    }

    /// Collect a bounded, read-only remote snapshot. Never expose a partial
    /// listing as complete or persist a cursor before all pages validate.
    pub fn snapshot(&self) -> Result<crate::DropboxDeltaPage, ReadError> {
        const MAX_PAGES: usize = 128;
        const MAX_ENTRIES: usize = 50_000;
        let mut cursor: Option<String> = None;
        let mut seen_cursors = std::collections::HashSet::new();
        let mut by_key = std::collections::BTreeMap::new();
        for _ in 0..MAX_PAGES {
            let page = self.list_delta(cursor.as_deref())?;
            if !seen_cursors.insert(page.cursor.clone()) {
                return Err(ReadError::Protocol(crate::Error::InvalidCursor));
            }
            for entry in page.entries {
                let key = match &entry {
                    crate::DropboxDeltaEntry::File(file) => file.logical_key.as_str(),
                    crate::DropboxDeltaEntry::Deleted(key) => key.as_str(),
                };
                by_key.insert(key.to_owned(), entry);
                if by_key.len() > MAX_ENTRIES {
                    return Err(ReadError::ResponseTooLarge);
                }
            }
            if !page.has_more {
                return Ok(crate::DropboxDeltaPage {
                    entries: by_key.into_values().collect(),
                    cursor: page.cursor,
                    has_more: false,
                });
            }
            cursor = Some(page.cursor);
        }
        Err(ReadError::ResponseTooLarge)
    }

    /// Lookup happens inside the verified account and vault scope. Caller
    /// cannot supply an unrelated Dropbox file reference or revision.
    pub fn read_key_verified(
        &self, key: &ObjectKey, expected_hash: &TrustedBlake3,
    ) -> Result<VerifiedRead, ReadError> {
        let metadata = self.metadata(key)?;
        self.download_verified(&metadata, expected_hash)
    }

    /// Internally pinned revision download, never an apply permission.
    fn download_verified(
        &self, file: &DropboxFileRef, expected_hash: &TrustedBlake3,
    ) -> Result<VerifiedRead, ReadError> {
        if file.byte_length > MAX_READ_BYTES as u64 {
            return Err(ReadError::Verify(VerifyError::TooLarge));
        }
        if !file.downloadable { return Err(ReadError::NoDownloadPermission); }
        let url = format!("{}/2/files/download", self.endpoints.content);
        let api_arg = pinned_download_arg(&file.revision);
        debug_assert!(api_arg.is_ascii());
        let response = self.agent.post(&url)
            .set("Authorization", &self.token.bearer())
            .set("Dropbox-API-Arg", &api_arg)
            .call().map_err(Self::from_ureq)?;
        let result = response.header("Dropbox-API-Result")
            .ok_or(ReadError::MissingResultHeader)?;
        if result.len() > MAX_METADATA_BYTES {
            return Err(ReadError::ResponseTooLarge);
        }
        let result = result.as_bytes().to_vec();
        let bytes = bounded(response, file.byte_length as usize)?;
        verify_download(&self.scope, &file.logical_key,
            file, &result, bytes, expected_hash).map_err(ReadError::Verify)
    }

    fn from_ureq(err: ureq::Error) -> ReadError {
        match err {
            ureq::Error::Status(status, response) => {
                let retry = response.header("Retry-After").map(ToOwned::to_owned);
                let bytes = bounded(response, 4 * 1024).unwrap_or_default();
                let err = classify_http(Operation::Read, status, &bytes, retry.as_deref())
                    .err().unwrap_or(crate::Error::UnexpectedResponse);
                ReadError::Protocol(err)
            }
            ureq::Error::Transport(_) => ReadError::NetworkUnavailable,
        }
    }
}

fn bounded(response: ureq::Response, max: usize) -> Result<Vec<u8>, ReadError> {
    if response.header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|size| size > max as u64)
    {
        return Err(ReadError::ResponseTooLarge);
    }
    let mut reader = response.into_reader().take(max as u64 + 1);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|_| ReadError::NetworkUnavailable)?;
    if bytes.len() > max { return Err(ReadError::ResponseTooLarge); }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
