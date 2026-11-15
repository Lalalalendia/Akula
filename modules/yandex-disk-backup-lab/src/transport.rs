//! Explicit short-lived OAuth-backed REST transport for encrypted app:/ backup
//! objects. This module has no native UI, scheduler or user-file access.
//! Never print the OAuth token or the signed one-time upload/download URLs.
//! Do not enable in installed Akula until a real disposable-account E2E,
//! recovery-key UX and local File Gate acceptance are independently proven.

use super::{
    BackupError, CreateOnlyBackupPort, UploadAcknowledgment, YandexAppPath,
    MAX_ENCRYPTED_BYTES, MAGIC, NONCE_BYTES, TAG_BYTES, valid_backup_path,
};
use serde::Deserialize;
use std::{io::Read, time::Duration};
use url::Url;
use zeroize::Zeroizing;

const FIXED_UPLOAD_LINK_API: &str =
    "https://cloud-api.yandex.net/v1/disk/resources/upload";
const FIXED_DOWNLOAD_LINK_API: &str =
    "https://cloud-api.yandex.net/v1/disk/resources/download";
const MAX_LINK_RESPONSE: usize = 16 * 1024;
const MAX_SIGNED_URL: usize = 16 * 1024;
const MAX_BEARER: usize = 4096;

#[derive(Clone, Copy, Eq, PartialEq)]
enum TransferKind { Upload, Download }

#[derive(Deserialize)]
struct WireLink {
    href: String,
    method: String,
    templated: bool,
}

fn allowed_yandex_host(host: &str, kind: TransferKind) -> bool {
    match kind {
        TransferKind::Upload => {
            host.starts_with("uploader")
                && host.len() > "uploader".len()
                && (host.ends_with(".dst.yandex.net")
                    || host.ends_with(".disk.yandex.net"))
        }
        TransferKind::Download => {
            host == "downloader.disk.yandex.ru"
                || host == "downloader.disk.yandex.net"
                || (host.ends_with(".storage.yandex.net")
                    && host.len() > ".storage.yandex.net".len())
        }
    }
}

/// Validate all signed links as **runtime ephemeral capabilities**. A URL from
/// a fixed Yandex metadata endpoint is still untrusted; no arbitrary host,
/// IP address, HTTP downgrade, alternate port, credential or redirect is
/// accepted. Return unmodified href to preserve Yandex's signature encoding.
fn parse_signed_link(
    body: &[u8], kind: TransferKind,
) -> Result<Zeroizing<String>, BackupError> {
    if body.len() > MAX_LINK_RESPONSE { return Err(BackupError::ProviderUnavailable); }
    let link: WireLink = serde_json::from_slice(body)
        .map_err(|_| BackupError::ProviderUnavailable)?;
    if link.templated
        || link.method != match kind {
            TransferKind::Upload => "PUT",
            TransferKind::Download => "GET",
        }
        || link.href.len() > MAX_SIGNED_URL
        || link.href.is_empty()
        || !link.href.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(BackupError::ProviderUnavailable);
    }
    let href = Zeroizing::new(link.href);
    let parsed = Url::parse(href.as_str())
        .map_err(|_| BackupError::ProviderUnavailable)?;
    let host = parsed.host_str().ok_or(BackupError::ProviderUnavailable)?;
    if parsed.scheme() != "https"
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty() || parsed.password().is_some()
        || parsed.fragment().is_some()
        || !allowed_yandex_host(host, kind)
        || (kind == TransferKind::Upload
            && !parsed.path().starts_with("/upload-target"))
    {
        return Err(BackupError::ProviderUnavailable);
    }
    Ok(href)
}

fn read_limited(
    response: ureq::Response, max: usize,
) -> Result<Vec<u8>, BackupError> {
    let mut result = Vec::new();
    response.into_reader()
        .take(max.saturating_add(1) as u64)
        .read_to_end(&mut result)
        .map_err(|_| BackupError::ProviderUnavailable)?;
    if result.len() > max {
        return Err(BackupError::RemoteObjectTooLarge);
    }
    Ok(result)
}

fn map_metadata_error(err: ureq::Error) -> BackupError {
    match err {
        ureq::Error::Status(409, _) => BackupError::PreexistingRemoteObject,
        ureq::Error::Status(400 | 401 | 403 | 412 | 413 | 507, _) =>
            BackupError::ProviderDenied,
        ureq::Error::Status(_, _) | ureq::Error::Transport(_) =>
            BackupError::ProviderUnavailable,
    }
}

pub struct YandexDiskRestBackupPort<'a> {
    access_token: &'a str,
    agent: ureq::Agent,
}

impl<'a> YandexDiskRestBackupPort<'a> {
    /// Pass an already verified *ephemeral* access token from explicit PKCE
    /// login; never retrieve or refresh a token inside this transport.
    pub fn new(access_token: &'a str) -> Result<Self, BackupError> {
        if access_token.is_empty() || access_token.len() > MAX_BEARER
            || !access_token.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(BackupError::ProviderDenied);
        }
        Ok(Self {
            access_token,
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(Duration::from_secs(30))
                .timeout_write(Duration::from_secs(30))
                .redirects(0)
                .build(),
        })
    }

    fn api_link(
        &self, path: &YandexAppPath, kind: TransferKind,
    ) -> Result<Option<Zeroizing<String>>, BackupError> {
        if !valid_backup_path(path) {
            return Err(BackupError::InvalidPath);
        }
        let endpoint = match kind {
            TransferKind::Upload => FIXED_UPLOAD_LINK_API,
            TransferKind::Download => FIXED_DOWNLOAD_LINK_API,
        };
        let authorization = Zeroizing::new(format!("OAuth {}", self.access_token));
        let mut req = self.agent.get(endpoint)
            .set("Authorization", authorization.as_str())
            .set("Accept", "application/json")
            .query("path", path.as_str());
        if kind == TransferKind::Upload {
            req = req.query("overwrite", "false");
        }
        match req.call() {
            Ok(response) if response.status() == 200 => {
                let json = read_limited(response, MAX_LINK_RESPONSE)?;
                Ok(Some(parse_signed_link(&json, kind)?))
            }
            Ok(_) => Err(BackupError::ProviderUnavailable),
            Err(ureq::Error::Status(404, _))
                if kind == TransferKind::Download => Ok(None),
            Err(err) => Err(map_metadata_error(err)),
        }
    }
}

impl CreateOnlyBackupPort for YandexDiskRestBackupPort<'_> {
    fn create_only(
        &mut self, path: &YandexAppPath, ciphertext: &[u8],
    ) -> Result<UploadAcknowledgment, BackupError> {
        // Defense in depth: the production transport refuses accidental raw
        // plaintext, although the authenticated sealing step remains the
        // real proof that archive bytes are cryptographically protected.
        if ciphertext.len() > MAX_ENCRYPTED_BYTES {
            return Err(BackupError::PayloadTooLarge);
        }
        if !ciphertext.starts_with(MAGIC)
            || ciphertext.len() < MAGIC.len() + NONCE_BYTES + TAG_BYTES + 1
        {
            return Err(BackupError::InvalidEnvelope);
        }
        let href = self.api_link(path, TransferKind::Upload)?
            .ok_or(BackupError::ProviderUnavailable)?;
        // The signed href is a short-lived capability: no OAuth header here,
        // no redirects, no manual retry on a failed/ambiguous PUT response.
        match self.agent.put(href.as_str())
            .set("Content-Type", "application/octet-stream")
            .send_bytes(ciphertext)
        {
            Ok(r) if r.status() == 201 => Ok(UploadAcknowledgment::Created201),
            Ok(r) if r.status() == 202 => Ok(UploadAcknowledgment::Pending202),
            Ok(_) => Err(BackupError::CommitUncertain),
            Err(ureq::Error::Status(409, _)) =>
                Err(BackupError::PreexistingRemoteObject),
            Err(ureq::Error::Status(413, _)) =>
                Err(BackupError::PayloadTooLarge),
            Err(ureq::Error::Status(_, _) | ureq::Error::Transport(_)) =>
                Err(BackupError::CommitUncertain),
        }
    }

    fn read_exact_bounded(
        &mut self, path: &YandexAppPath, max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, BackupError> {
        let href = match self.api_link(path, TransferKind::Download)? {
            Some(href) => href,
            None => return Ok(None),
        };
        let response = match self.agent.get(href.as_str())
            .set("Accept", "application/octet-stream").call()
        {
            Ok(r) if r.status() == 200 => r,
            Ok(_) => return Err(BackupError::ProviderUnavailable),
            Err(_) => return Err(BackupError::ProviderUnavailable),
        };
        read_limited(response, max_bytes.min(MAX_ENCRYPTED_BYTES)).map(Some)
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
