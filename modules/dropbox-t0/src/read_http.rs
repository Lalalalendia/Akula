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
mod tests {
    use super::*;
    use crate::{DropboxContentHash, DropboxRevision};
    use crate::read_verify::dropbox_content_hash;
    use std::{
        io::{Read, Write}, net::TcpListener,
        thread,
    };

    fn scope() -> Scope {
        Scope::new("dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "home").unwrap()
    }

    // Fake Dropbox replies on loopback. Production cannot select this endpoint.
    fn server(contents: Vec<u8>, wrong_bytes: bool, redirect: bool)
        -> (Endpoints, thread::JoinHandle<()>)
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for i in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = Vec::new();
                let mut buf = [0_u8; 4096];
                let header_end;
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(p) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = p + 4;
                        break;
                    }
                }
                let header = std::str::from_utf8(&request[..header_end]).unwrap().to_owned();
                assert!(header.starts_with("POST /2/"));
                assert!(header.to_ascii_lowercase().contains("authorization: bearer fake-token"));
                let size: usize = header.lines()
                    .find_map(|row| row.to_ascii_lowercase().strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())).unwrap_or(0);
                while request.len() - header_end < size {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }

                let hash = dropbox_content_hash(&contents);
                let record = json!({
                    ".tag":"file", "id":"id:abc", "rev":"a1c10ce0dd78",
                    "size":contents.len(), "is_downloadable":true,
                    "content_hash":hash,
                }).to_string();
                let (body, extra) = match i {
                    0 => {
                        assert!(header.contains("/2/users/get_current_account"));
                        (br#"{"account_id":"dbid:alice","root_info":{"root_namespace_id":"12345"}}"#.to_vec(), String::new())
                    },
                    1 => {
                        assert!(header.contains("/2/files/get_metadata"));
                        (record.clone().into_bytes(), String::new())
                    },
                    _ => {
                        assert!(header.contains("/2/files/download"));
                        assert!(header.contains("rev:a1c10ce0dd78"));
                        let bytes = if wrong_bytes { b"invalid".to_vec() } else { contents.clone() };
                        (bytes, format!("Dropbox-API-Result: {}\r\n", record))
                    },
                };
                if redirect && i == 2 {
                    let msg = "HTTP/1.1 302 Found\r\nLocation: https://example.org/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    stream.write_all(msg.as_bytes()).unwrap();
                } else {
                    write!(stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
                        body.len(), extra).unwrap();
                    stream.write_all(&body).unwrap();
                }
            }
        });
        let base = format!("http://{addr}");
        (Endpoints { api: base.clone(), content: base }, handle)
    }

    fn connect_test(endpoints: Endpoints) -> DropboxReadOnly {
        DropboxReadOnly::connect_at(scope(),
            AccessToken::new("fake-token".to_owned()).unwrap(), endpoints).unwrap()
    }

    #[test]
    fn list_and_continue_use_only_verified_account_and_read_routes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            for i in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                let header_end = loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(p) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                };
                let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
                assert!(headers.to_ascii_lowercase().contains("authorization: bearer fake-token"));
                let size = headers.lines().find_map(|line|
                    line.to_ascii_lowercase().strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())).unwrap_or(0);
                while request.len() - header_end < size {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                let body = &request[header_end..header_end + size];
                let reply = match i {
                    0 => {
                        assert!(headers.contains("/2/users/get_current_account"));
                        json!({"account_id":"dbid:alice",
                            "root_info":{"root_namespace_id":"12345"}})
                    }
                    1 => {
                        assert!(headers.contains("/2/files/list_folder "));
                        let payload: Value = serde_json::from_slice(body).unwrap();
                        assert_eq!(payload["recursive"], true);
                        assert_eq!(payload["path"], format!(
                            "/akula-sync/v1/vaults/{}",scope().vault_id()));
                        json!({"entries":[],"cursor":"c1","has_more":true})
                    }
                    _ => {
                        assert!(headers.contains("/2/files/list_folder/continue "));
                        let payload: Value = serde_json::from_slice(body).unwrap();
                        assert_eq!(payload["cursor"], "c1");
                        json!({"entries":[],"cursor":"c2","has_more":false})
                    }
                }.to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.len(),reply).unwrap();
            }
        });
        let base = format!("http://{addr}");
        let client = connect_test(Endpoints { api: base.clone(), content: base });
        let first = client.list_delta(None).unwrap();
        assert!(first.entries.is_empty());
        assert!(first.has_more);
        let next = client.list_delta(Some(&first.cursor)).unwrap();
        assert_eq!(next.cursor,"c2");
        assert!(!next.has_more);
        worker.join().unwrap();
    }

    #[test]
    fn read_exact_revision_with_two_hashes_and_account_proof() {
        let bytes = b"hello akula".to_vec();
        let (endpoints, thread) = server(bytes.clone(), false, false);
        let client = connect_test(endpoints);
        let file = client.metadata(&ObjectKey::head()).unwrap();
        assert_eq!(file.revision, DropboxRevision::parse("a1c10ce0dd78").unwrap());
        assert_eq!(file.dropbox_hash, Some(DropboxContentHash::parse(
            &dropbox_content_hash(&bytes)).unwrap()));
        let trusted = TrustedBlake3::parse(
            &blake3::hash(&bytes).to_hex().to_string()).unwrap();
        let downloaded = client.download_verified(&file, &trusted).unwrap();
        assert_eq!(downloaded.bytes, bytes);
        thread.join().unwrap();
    }

    #[test]
    fn same_length_changed_payload_does_not_verify() {
        let (endpoints, thread) = server(b"invalid".to_vec(), true, false);
        let client = connect_test(endpoints);
        let file = client.metadata(&ObjectKey::head()).unwrap();
        let trusted = TrustedBlake3::parse(
            &blake3::hash(b"original").to_hex().to_string()).unwrap();
        let result = client.download_verified(&file, &trusted);
        assert_eq!(result, Err(ReadError::Verify(VerifyError::TrustedHashMismatch)));
        thread.join().unwrap();
    }

    #[test]
    fn public_read_key_cannot_bypass_scoped_metadata_lookup() {
        let payload = b"protected-by-trusted-anchor".to_vec();
        let (endpoints, server) = server(payload.clone(), false, false);
        let client = connect_test(endpoints);
        let trusted = TrustedBlake3::parse(
            &blake3::hash(&payload).to_hex().to_string()).unwrap();
        let verified = client.read_key_verified(&ObjectKey::head(), &trusted).unwrap();
        assert_eq!(verified.bytes, payload);
        server.join().unwrap();
    }

    #[test]
    fn redirects_are_rejected_without_leaking_token() {
        let (endpoints, thread) = server(b"original".to_vec(), false, true);
        let client = connect_test(endpoints);
        let file = client.metadata(&ObjectKey::head()).unwrap();
        let trusted = TrustedBlake3::parse(
            &blake3::hash(b"original").to_hex().to_string()).unwrap();
        assert!(client.download_verified(&file, &trusted).is_err());
        assert_eq!(format!("{:?}", client.token), "AccessToken(REDACTED)");
        thread.join().unwrap();
    }
}
