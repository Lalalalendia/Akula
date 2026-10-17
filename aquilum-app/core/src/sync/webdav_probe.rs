//! STOR-T-10/11: fail-closed, read-only WebDAV/Nextcloud discovery evidence.
//!
//! This module parses responses supplied by a trusted transport. It does NOT
//! perform network requests, authenticate, parse PROPFIND XML, or implement
//! RemoteStore. An OPTIONS/OCS advertisement is never proof of atomic writes,
//! a consistent remote snapshot, or safe two-client synchronization.

use super::ProviderCapabilities;

const MAX_HEADER_BYTES: usize = 4096;
const MAX_OCS_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebDavProbeError {
    AuthenticationRequired,
    PermissionDenied,
    RootMissing,
    RateLimited,
    Unavailable,
    RedirectRefused,
    UnsupportedResponse,
    InvalidHeader,
    InvalidOcsResponse,
    ResponseTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebDavAdvertisedFeatures {
    pub class_one: bool,
    pub class_two: bool,
    pub class_three: bool,
    pub sync_collection: bool,
    pub propfind: bool,
    pub put: bool,
    pub move_resource: bool,
    pub delete: bool,
    pub lock: bool,
}

impl WebDavAdvertisedFeatures {
    /// HTTP advertisements are only hints. ProviderCapabilities must be
    /// earned by testing the corresponding server-enforced operations.
    pub fn unverified_remote_capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            conditional_create: false,
            conditional_replace: false,
            conditional_delete: false,
            snapshot_consistency: false,
            version_history: false,
            delta_feed: false,
        }
    }
}

/// Parse an OPTIONS response for the selected, authenticated DAV collection.
///
/// The caller must have independently checked the endpoint, TLS, redirects,
/// authentication, selected vault root and response-size bounds. In particular,
/// a DAV: sync-collection token is NOT evidence that a REPORT cursor was read.
pub fn parse_webdav_options(
    status: u16,
    dav: Option<&str>,
    allow: Option<&str>,
) -> Result<WebDavAdvertisedFeatures, WebDavProbeError> {
    require_status(status, &[200, 204])?;

    let dav_tokens = parse_tokens(dav.ok_or(WebDavProbeError::InvalidHeader)?)?;
    let allow_tokens = match allow {
        Some(value) => parse_tokens(value)?,
        None => Vec::new(),
    };
    let has = |candidate: &str| dav_tokens.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    let method = |candidate: &str| allow_tokens.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    let class_one = has("1");
    let class_two = has("2");
    let class_three = has("3");
    if !class_one && !class_two && !class_three {
        return Err(WebDavProbeError::UnsupportedResponse);
    }

    Ok(WebDavAdvertisedFeatures {
        class_one,
        class_two,
        class_three,
        sync_collection: has("sync-collection"),
        propfind: method("PROPFIND"),
        put: method("PUT"),
        move_resource: method("MOVE"),
        delete: method("DELETE"),
        lock: method("LOCK"),
    })
}

fn parse_tokens(value: &str) -> Result<Vec<&str>, WebDavProbeError> {
    if value.len() > MAX_HEADER_BYTES
        || value.bytes().any(|b| b < 0x20 || b == 0x7f)
    {
        return Err(WebDavProbeError::InvalidHeader);
    }
    let tokens: Vec<_> = value.split(',').map(str::trim).collect();
    if tokens.iter().any(|token| token.is_empty()) {
        return Err(WebDavProbeError::InvalidHeader);
    }
    Ok(tokens)
}

/// A 207 response is only an HTTP envelope. The XML multistatus content,
/// per-resource propstat errors, namespaces and href scope MUST be validated
/// before constructing a RemoteSnapshot. This module deliberately cannot do so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DavRootProbe {
    MultistatusXmlValidationRequired,
}

pub fn classify_root_propfind(status: u16) -> Result<DavRootProbe, WebDavProbeError> {
    require_status(status, &[207])?;
    Ok(DavRootProbe::MultistatusXmlValidationRequired)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OcsAdvertisement {
    /// These flags mean that OCS advertised a capability group, not that an
    /// individual operation was tested or granted write authority.
    pub files_group: bool,
    pub dav_group: bool,
}

/// An optional Nextcloud/ownCloud OCS capabilities response.
///
/// HTTP 404/405 means the optional OCS endpoint is not advertised. All other
/// errors are distinguished; a malformed 200 response is never silently
/// converted into "generic WebDAV" or an empty remote directory.
pub fn parse_ocs_capabilities(
    status: u16,
    body: &[u8],
) -> Result<Option<OcsAdvertisement>, WebDavProbeError> {
    if status == 404 || status == 405 {
        return Ok(None);
    }
    require_status(status, &[200])?;
    if body.len() > MAX_OCS_BYTES {
        return Err(WebDavProbeError::ResponseTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| WebDavProbeError::InvalidOcsResponse)?;
    let meta = value.pointer("/ocs/meta").and_then(serde_json::Value::as_object)
        .ok_or(WebDavProbeError::InvalidOcsResponse)?;
    let success = meta.get("status").and_then(serde_json::Value::as_str)
        .is_some_and(|text| text.eq_ignore_ascii_case("ok"));
    let accepted_status = matches!(
        meta.get("statuscode").and_then(serde_json::Value::as_u64),
        Some(100 | 200)
    );
    if !success || !accepted_status {
        return Err(WebDavProbeError::InvalidOcsResponse);
    }
    let capabilities = value.pointer("/ocs/data/capabilities")
        .and_then(serde_json::Value::as_object)
        .ok_or(WebDavProbeError::InvalidOcsResponse)?;
    Ok(Some(OcsAdvertisement {
        files_group: capabilities.get("files").is_some_and(serde_json::Value::is_object),
        dav_group: capabilities.get("dav").is_some_and(serde_json::Value::is_object),
    }))
}

fn require_status(status: u16, allowed: &[u16]) -> Result<(), WebDavProbeError> {
    if allowed.contains(&status) {
        return Ok(());
    }
    Err(match status {
        301 | 302 | 303 | 307 | 308 => WebDavProbeError::RedirectRefused,
        401 => WebDavProbeError::AuthenticationRequired,
        403 => WebDavProbeError::PermissionDenied,
        404 => WebDavProbeError::RootMissing,
        429 => WebDavProbeError::RateLimited,
        500..=599 => WebDavProbeError::Unavailable,
        _ => WebDavProbeError::UnsupportedResponse,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webdav_options_are_advertisements_not_write_permission() {
        let found = parse_webdav_options(
            200,
            Some("1, 2, sync-collection"),
            Some("GET, HEAD, PROPFIND, PUT, MOVE, DELETE, LOCK"),
        ).unwrap();
        assert!(found.class_one && found.class_two);
        assert!(found.sync_collection && found.propfind && found.put);
        assert!(found.move_resource && found.delete && found.lock);
        let unverified = found.unverified_remote_capabilities();
        assert!(!unverified.conditional_create);
        assert!(!unverified.conditional_replace);
        assert!(!unverified.conditional_delete);
        assert!(!unverified.snapshot_consistency);
        assert!(!unverified.version_history);
        assert!(!unverified.delta_feed);
    }

    #[test]
    fn exact_tokens_do_not_grant_features_by_substring() {
        let found = parse_webdav_options(
            204, Some("3"), Some("SUPERPROPFIND, OUTPUT, REMOVED, UNLOCK"),
        ).unwrap();
        assert!(found.class_three);
        assert!(!found.propfind && !found.put);
        assert!(!found.move_resource && !found.lock);
        assert!(!found.delete);
        assert!(!found.sync_collection);
    }

    #[test]
    fn missing_dav_advertisement_and_invalid_headers_are_rejected() {
        assert_eq!(
            parse_webdav_options(200, None, Some("PROPFIND")),
            Err(WebDavProbeError::InvalidHeader)
        );
        assert_eq!(
            parse_webdav_options(200, Some("11, sync-collection"), Some("PUT")),
            Err(WebDavProbeError::UnsupportedResponse)
        );
        for bad in ["1,,2", "1\nX-Header: yes", "1\r\n", &"A".repeat(4097)] {
            assert_eq!(
                parse_webdav_options(200, Some(bad), None),
                Err(WebDavProbeError::InvalidHeader),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse_webdav_options(200, Some("1"), Some("GET, \r\nPUT")),
            Err(WebDavProbeError::InvalidHeader)
        );
    }

    #[test]
    fn options_fail_closed_on_transport_and_authentication_states() {
        for (status, expected) in [
            (301, WebDavProbeError::RedirectRefused),
            (302, WebDavProbeError::RedirectRefused),
            (401, WebDavProbeError::AuthenticationRequired),
            (403, WebDavProbeError::PermissionDenied),
            (404, WebDavProbeError::RootMissing),
            (405, WebDavProbeError::UnsupportedResponse),
            (429, WebDavProbeError::RateLimited),
            (503, WebDavProbeError::Unavailable),
        ] {
            assert_eq!(
                parse_webdav_options(status, Some("1"), Some("PROPFIND")),
                Err(expected),
                "status={status}"
            );
        }
    }

    #[test]
    fn propfind_http_envelope_never_claims_verified_empty_listing() {
        assert_eq!(
            classify_root_propfind(207),
            Ok(DavRootProbe::MultistatusXmlValidationRequired)
        );
        assert_eq!(classify_root_propfind(404), Err(WebDavProbeError::RootMissing));
        assert_eq!(classify_root_propfind(403), Err(WebDavProbeError::PermissionDenied));
        assert_eq!(classify_root_propfind(200), Err(WebDavProbeError::UnsupportedResponse));
    }

    #[test]
    fn valid_ocs_advertises_groups_but_not_conditional_mutation() {
        let sample = br#"{
            "ocs": {
                "meta": {"status": "ok", "statuscode": 200},
                "data": {"capabilities": {"files": {"versioning": true}, "dav": {"chunking": "1.0"}}}
            }
        }"#;
        assert_eq!(
            parse_ocs_capabilities(200, sample),
            Ok(Some(OcsAdvertisement { files_group: true, dav_group: true }))
        );
        // The capabilities JSON is not evidence of server-enforced If-Match.
        let options = parse_webdav_options(200, Some("1,2"), Some("PUT")).unwrap();
        assert!(!options.unverified_remote_capabilities().supports_guarded_mutation());
    }

    #[test]
    fn old_ocs_success_code_is_accepted_without_implied_features() {
        let sample = br#"{
            "ocs": {"meta": {"status": "ok", "statuscode": 100},
                    "data": {"capabilities": {}}}
        }"#;
        assert_eq!(
            parse_ocs_capabilities(200, sample),
            Ok(Some(OcsAdvertisement { files_group: false, dav_group: false }))
        );
    }

    #[test]
    fn unavailable_ocs_is_optional_but_auth_and_server_errors_are_not() {
        assert_eq!(parse_ocs_capabilities(404, b""), Ok(None));
        assert_eq!(parse_ocs_capabilities(405, b""), Ok(None));
        assert_eq!(
            parse_ocs_capabilities(401, b""),
            Err(WebDavProbeError::AuthenticationRequired)
        );
        assert_eq!(
            parse_ocs_capabilities(503, b""),
            Err(WebDavProbeError::Unavailable)
        );
        assert_eq!(
            parse_ocs_capabilities(302, b""),
            Err(WebDavProbeError::RedirectRefused)
        );
    }

    #[test]
    fn malformed_and_oversized_ocs_are_never_downgraded_silently() {
        for body in [
            &br#"{}"#[..],
            &br#"{"ocs":{"meta":{"status":"ok","statuscode":200}}}"#[..],
            &br#"{"ocs":{"meta":{"status":"failed","statuscode":200},"data":{"capabilities":{}}}}"#[..],
            &br#"{"ocs":{"meta":{"status":"ok","statuscode":200},"data":{"capabilities":[]}}}"#[..],
            &br#"not-json"#[..],
        ] {
            assert_eq!(
                parse_ocs_capabilities(200, body),
                Err(WebDavProbeError::InvalidOcsResponse)
            );
        }
        assert_eq!(
            parse_ocs_capabilities(200, &vec![b'x'; MAX_OCS_BYTES + 1]),
            Err(WebDavProbeError::ResponseTooLarge)
        );
    }
}
