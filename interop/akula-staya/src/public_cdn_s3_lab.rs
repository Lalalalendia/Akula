//! Non-shipping, strictly read-only S3-compatible HTTP Range adapter.
//!
//! Production construction requires an HTTPS URL independently approved by
//! the publisher/catalog. Local MinIO HTTP is enabled only by an explicit
//! test feature. No SDK credentials, PUT, DELETE, LIST, redirects, or proxy
//! credentials may be sourced from untrusted peers.
use crate::public_cdn_lab::{
    ChunkRequest, ImmutableRangeOrigin, OriginReply, PublisherPinnedManifest,
};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_RANGE, IF_MATCH, RANGE};
use std::{io::Read, time::Duration};

pub struct PinnedS3Range {
    client: Client,
    url: reqwest::Url,
    revision_etag: Option<String>,
}

fn safe_url(value: &str, allow_loopback_http: bool) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(value).map_err(|_| "invalid object URL")?;
    let https = parsed.scheme() == "https";
    let local = allow_loopback_http
        && parsed.scheme() == "http"
        && parsed.host_str() == Some("127.0.0.1")
        && parsed.port().is_some();
    if (!https && !local)
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.path().is_empty()
        || parsed.path() == "/"
        || parsed.path_segments().is_none()
        || parsed.path_segments().is_some_and(|s| s.into_iter().any(|p| p == "." || p == ".."))
    {
        return Err("unsafe S3 object URL".to_owned());
    }
    Ok(parsed)
}

impl PinnedS3Range {
    pub fn new_https(
        manifest: &PublisherPinnedManifest,
        independently_approved_url: &str,
        version_etag: Option<&str>,
    ) -> Result<Self, String> {
        Self::build(manifest, independently_approved_url, version_etag, false)
    }

    /// A guarded MinIO-only path, deliberately unavailable to shipping builds.
    #[cfg(feature = "public-cdn-local-minio-test")]
    pub fn for_test_loopback_minio(
        manifest: &PublisherPinnedManifest,
        url: &str,
    ) -> Result<Self, String> {
        Self::build(manifest, url, None, true)
    }

    fn build(
        manifest: &PublisherPinnedManifest,
        independently_approved_url: &str,
        version_etag: Option<&str>,
        allow_loopback_http: bool,
    ) -> Result<Self, String> {
        // A signed/pinned manifest cannot silently redirect to a foreign origin.
        if manifest.origin_url_for_allowlisted_adapter() != independently_approved_url {
            return Err("publisher-pinned origin does not match approved URL".into());
        }
        let url = safe_url(independently_approved_url, allow_loopback_http)?;
        // An ETag is ONLY an opaque version precondition, never a content hash.
        let revision_etag = match version_etag {
            None => None,
            Some(v) if v.len() <= 256 && !v.is_empty()
                && v.as_bytes().iter().all(|b| (0x21..=0x7e).contains(b))
                && !v.contains(['\r', '\n']) => Some(v.to_owned()),
            _ => return Err("invalid opaque version precondition".into()),
        };
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| "HTTP client configuration denied")?;
        Ok(Self { client, url, revision_etag })
    }

    fn read_exact_bounded(&mut self, request: &ChunkRequest<'_>) -> Result<OriginReply, String> {
        if request.length == 0 || request.length > 4 * 1024 * 1024
            || request.offset.checked_add(request.length as u64)
                .is_none_or(|end| end > request.object_size)
        {
            return Err("invalid bounded Range request".into());
        }
        let end = request.offset + request.length as u64 - 1;
        let range = format!("bytes={}-{}", request.offset, end);
        let mut get = self.client.get(self.url.clone())
            .header(RANGE, range)
            .header(ACCEPT_ENCODING, "identity")
            .header(CACHE_CONTROL, "no-store");
        if let Some(etag) = &self.revision_etag {
            get = get.header(IF_MATCH, etag.as_str());
        }
        let response = get.send().map_err(|e| format!("S3 Range request failed: {e}"))?;
        let status = response.status().as_u16();
        // Do not follow redirects, accept a full-file 200, or read an error body.
        if status != 206 {
            return Err(format!("S3 rejected exact ranged GET with HTTP {status}"));
        }
        let read_header = |key: reqwest::header::HeaderName| -> Option<String> {
            response.headers().get(key)?.to_str().ok().map(str::to_owned)
        };
        let expected_range = format!("bytes {}-{end}/{}", request.offset, request.object_size);
        let content_range = read_header(CONTENT_RANGE);
        let content_encoding = read_header(CONTENT_ENCODING);
        let content_length = response.content_length();
        if content_range.as_deref() != Some(expected_range.as_str())
            || content_length != Some(request.length as u64)
            || content_encoding.as_deref().is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
        {
            return Err("untrusted S3 HTTP range metadata".into());
        }
        let mut bytes = Vec::with_capacity(request.length);
        let limit = request.length as u64 + 1;
        response.take(limit).read_to_end(&mut bytes)
            .map_err(|e| format!("bounded S3 body read: {e}"))?;
        if bytes.len() != request.length {
            return Err("S3 response size did not match requested range".into());
        }
        Ok(OriginReply {
            status,
            content_range,
            content_length,
            content_encoding,
            bytes,
        })
    }
}

impl ImmutableRangeOrigin for PinnedS3Range {
    fn get_range(&mut self, request: &ChunkRequest<'_>) -> Result<OriginReply, ()> {
        self.read_exact_bounded(request).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_plain_http_and_untrusted_userinfo() {
        assert!(safe_url("http://example.com/a", false).is_err());
        assert!(safe_url("http://127.0.0.1:9000/a", false).is_err());
        assert!(safe_url("https://admin:pass@example.com/a", false).is_err());
        assert!(safe_url("file:///etc/passwd", false).is_err());
        assert!(safe_url("https://example.com/a#fragment", false).is_err());
        assert!(safe_url("https://example.com/immutable/a", false).is_ok());
    }
}
