//! Offline S3 V1 connection profile and typed errors.
//!
//! This module is compiled by the `sync-s3-invite` feature without AWS SDK,
//! Tokio, File Gate custody, credentials discovery or local/remote writes.
//! Endpoint restrictions are identical for Windows transport and Android.
//! A syntactically valid profile does NOT authenticate the remote owner.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3TransportError {
    InvalidProfile,
    UnsafeEndpoint,
    OutsideVaultNamespace,
    NotFound,
    PermissionDenied,
    PreconditionFailed,
    CommitUncertain,
    Unavailable,
    MalformedResponse,
    ObjectTooLarge,
    ChangedDuringRead,
    IncompleteListing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3ConnectionProfile {
    pub bucket: String,
    pub region: String,
    /// None selects the standard AWS endpoint for the configured region.
    pub endpoint: Option<String>,
    pub force_path_style: bool,
}

impl S3ConnectionProfile {
    pub fn validate(&self) -> Result<(), S3TransportError> {
        let name = self.bucket.as_bytes();
        if !(3..=63).contains(&name.len())
            || !name.first().is_some_and(u8::is_ascii_lowercase)
                && !name.first().is_some_and(u8::is_ascii_digit)
            || !name.last().is_some_and(u8::is_ascii_lowercase)
                && !name.last().is_some_and(u8::is_ascii_digit)
            || !name.iter().all(|ch| ch.is_ascii_lowercase()
                || ch.is_ascii_digit() || *ch == b'-' || *ch == b'.')
            || self.bucket.contains("..")
        {
            return Err(S3TransportError::InvalidProfile);
        }
        if self.region.is_empty() || self.region.len() > 64
            || !self.region.bytes().all(|ch| ch.is_ascii_lowercase()
                || ch.is_ascii_digit() || ch == b'-')
        {
            return Err(S3TransportError::InvalidProfile);
        }
        if let Some(endpoint) = &self.endpoint {
            let url = url::Url::parse(endpoint).map_err(|_| S3TransportError::UnsafeEndpoint)?;
            if url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(S3TransportError::UnsafeEndpoint);
            }
            match url.scheme() {
                "https" => {}
                "http" if self.force_path_style
                    && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")) => {}
                _ => return Err(S3TransportError::UnsafeEndpoint),
            }
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(endpoint: Option<&str>, path_style: bool) -> S3ConnectionProfile {
        S3ConnectionProfile {
            bucket: "akula-private".into(), region: "ru-central1".into(),
            endpoint: endpoint.map(str::to_string), force_path_style: path_style,
        }
    }

    #[test]
    fn portable_connection_profile_accepts_only_safe_endpoints() {
        assert_eq!(candidate(Some("https://storage.yandexcloud.net"), true).validate(), Ok(()));
        assert_eq!(candidate(None, false).validate(), Ok(()));
        assert_eq!(candidate(Some("http://127.0.0.1:9000"), true).validate(), Ok(()));
        for bad in [
            "http://storage.yandexcloud.net",
            "https://user:pass@example.net",
            "https://example.net/a",
            "https://example.net/?access_key=secret",
            "file:///tmp/s3",
        ] {
            assert_eq!(candidate(Some(bad), true).validate(), Err(S3TransportError::UnsafeEndpoint));
        }
        assert_eq!(candidate(Some("http://127.0.0.1:9000"), false).validate(),
            Err(S3TransportError::UnsafeEndpoint));
    }
}
