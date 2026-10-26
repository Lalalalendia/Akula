//! Explicit read-only Dropbox reconnect after application restart.
//! Only OS vault -> short-lived token -> verified account/root/head.
//! Neither an OAuth refresh nor a matching BLAKE3 grants publisher trust,
//! File Gate rights, mutation, upload, delete or ConfirmedBaseline.

use super::{credential_scope, ReadOnlyHeadCandidate};
use akula_dropbox_auth_lab::{READ_SCOPES, TOKEN_ENDPOINT};
use akula_dropbox_secret_store::{CredentialScope, CredentialVault, RefreshToken, SecretError};
use akula_dropbox_t0::{
    read_http::{AccessToken, DropboxReadOnly, ReadError},
    read_verify::{TrustedBlake3, VerifiedRead, MAX_READ_BYTES},
    ObjectKey, Scope,
};
use serde::Deserialize;
use std::{io::Read, time::Duration};
use url::form_urlencoded;
use zeroize::{Zeroize, Zeroizing};

const MAX_TOKEN_REPLY: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    InvalidOAuthApp, SecretStore(SecretError), MissingConsent,
    RefreshRejected, RefreshUncertain, ResponseTooLarge,
    MalformedResponse, ScopeEscalation, Remote(ReadError),
    WrongLogicalHead, WrongPinnedBytes, Oversize,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Seconds { Number(u64), Text(String) }
impl Seconds {
    fn checked(&self) -> Option<u64> {
        let n = match self {
            Self::Number(n) => *n,
            Self::Text(s) => s.parse().ok()?,
        };
        (1..=86_400).contains(&n).then_some(n)
    }
}

/// Dropbox normally returns no refresh_token or account_id on refresh.
/// Because we request the scope field explicitly, absence is a hard failure.
/// Reject unexpected JSON properties, including a surprise refresh_token.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefreshReply {
    access_token: String,
    token_type: String,
    expires_in: Seconds,
    scope: String,
}
impl Drop for RefreshReply {
    fn drop(&mut self) { self.access_token.zeroize(); }
}

pub(super) struct ShortLivedAccess {
    pub(super) token: AccessToken,
    #[allow(dead_code)]
    expires_in_seconds: u64,
}

pub(super) fn refresh_once(app_key: &str, saved: &RefreshToken)
    -> Result<ShortLivedAccess, ResumeError>
{
    refresh_at(TOKEN_ENDPOINT, app_key, saved)
}

/// A test can substitute a local endpoint only inside this private module.
fn refresh_at(endpoint: &str, app_key: &str, saved: &RefreshToken)
    -> Result<ShortLivedAccess, ResumeError>
{
    let form = Zeroizing::new(
        form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("refresh_token", saved.as_str())
            .append_pair("client_id", app_key)
            .append_pair("scope", READ_SCOPES)
            .finish(),
    );
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(20))
        .build();
    let response = match agent.post(endpoint)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .set("Accept", "application/json")
        .send_string(&form)
    {
        Ok(x) => x,
        Err(ureq::Error::Status(code, _)) if (400..500).contains(&code) =>
            return Err(ResumeError::RefreshRejected),
        Err(_) => return Err(ResumeError::RefreshUncertain),
    };
    if response.header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|n| n > MAX_TOKEN_REPLY as u64)
    {
        return Err(ResumeError::ResponseTooLarge);
    }
    let mut raw = Zeroizing::new(Vec::new());
    response.into_reader().take((MAX_TOKEN_REPLY + 1) as u64)
        .read_to_end(&mut raw).map_err(|_| ResumeError::RefreshUncertain)?;
    if raw.len() > MAX_TOKEN_REPLY { return Err(ResumeError::ResponseTooLarge); }
    let mut reply: RefreshReply =
        serde_json::from_slice(&raw).map_err(|_| ResumeError::MalformedResponse)?;
    if !reply.token_type.eq_ignore_ascii_case("bearer")
        || reply.access_token.is_empty() || reply.access_token.len() > 4096
        || !reply.access_token.bytes().all(|b|
            b.is_ascii_graphic() && b != b'"' && b != b'\\')
    {
        return Err(ResumeError::MalformedResponse);
    }
    let lifetime = reply.expires_in.checked()
        .ok_or(ResumeError::MalformedResponse)?;
    // Compare multisets, not just membership and count: a duplicate scope
    // must never stand in for a required scope that is absent.
    let mut requested: Vec<&str> = READ_SCOPES.split_ascii_whitespace().collect();
    let mut granted: Vec<&str> = reply.scope.split_ascii_whitespace().collect();
    requested.sort_unstable();
    granted.sort_unstable();
    if granted != requested {
        return Err(ResumeError::ScopeEscalation);
    }
    let access = AccessToken::new(std::mem::take(&mut reply.access_token))
        .map_err(|_| ResumeError::MalformedResponse)?;
    Ok(ShortLivedAccess { token: access, expires_in_seconds: lifetime })
}

/// Mockable inside this module only. Production always uses system keyring,
/// official HTTPS Dropbox OAuth endpoint and exact scoped DropboxReadOnly.
trait ResumePorts {
    fn load(&self, key: &CredentialScope) -> Result<Option<RefreshToken>, ResumeError>;
    fn refresh(&self, app: &str, token: &RefreshToken)
        -> Result<ShortLivedAccess, ResumeError>;
    fn read_head(&self, scope: &Scope, token: AccessToken, pin: &TrustedBlake3)
        -> Result<VerifiedRead, ResumeError>;
}
struct LivePorts;
impl ResumePorts for LivePorts {
    fn load(&self, key: &CredentialScope) -> Result<Option<RefreshToken>, ResumeError> {
        let vault = CredentialVault::system().map_err(ResumeError::SecretStore)?;
        vault.load_refresh(key).map_err(ResumeError::SecretStore)
    }
    fn refresh(&self, app: &str, token: &RefreshToken)
        -> Result<ShortLivedAccess, ResumeError> {
        refresh_once(app, token)
    }
    fn read_head(&self, scope: &Scope, token: AccessToken, pin: &TrustedBlake3)
        -> Result<VerifiedRead, ResumeError> {
        // Refresh replies cannot prove identity. This API separately calls
        // get_current_account, checks account/root and exact revision+hash.
        let client = DropboxReadOnly::connect(scope.clone(), token)
            .map_err(ResumeError::Remote)?;
        client.read_key_verified(&ObjectKey::head(), pin).map_err(ResumeError::Remote)
    }
}

/// Explicit opt-in entrypoint. No startup hooks or scheduled refresh.
pub fn resume_readonly(
    scope: Scope, oauth_app_key: &str, independently_pinned_head: TrustedBlake3,
) -> Result<ReadOnlyHeadCandidate, ResumeError> {
    resume_with(&LivePorts, &scope, oauth_app_key, &independently_pinned_head)
}
fn resume_with<P: ResumePorts>(
    ports: &P, scope: &Scope, app: &str, pin: &TrustedBlake3,
) -> Result<ReadOnlyHeadCandidate, ResumeError> {
    let key = credential_scope(scope, app).map_err(|_| ResumeError::InvalidOAuthApp)?;
    let saved = ports.load(&key)?.ok_or(ResumeError::MissingConsent)?;
    let access = ports.refresh(app, &saved)?;
    let head = ports.read_head(scope, access.token, pin)?;
    if head.logical_key != ObjectKey::head() {
        return Err(ResumeError::WrongLogicalHead);
    }
    if head.bytes.len() > MAX_READ_BYTES { return Err(ResumeError::Oversize); }
    if blake3::hash(&head.bytes).to_hex().as_str() != pin.as_str() {
        return Err(ResumeError::WrongPinnedBytes);
    }
    Ok(ReadOnlyHeadCandidate {
        scope: scope.clone(), revision: head.revision, bytes: head.bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConnectionStatus;
    use akula_dropbox_t0::DropboxRevision;
    use std::{collections::HashMap, io::{Read,Write},net::TcpListener,sync::Mutex,thread};

    const HEAD: &[u8] = b"synthetic pinned root after restart";
    fn scope() -> Scope {
        Scope::new("dbid:alice","12345",
            "12345678-1234-1234-1234-123456789abc","profile").unwrap()
    }
    fn pin() -> TrustedBlake3 {
        TrustedBlake3::parse(&blake3::hash(HEAD).to_hex().to_string()).unwrap()
    }
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<&'static str>>,
        missing: bool, store_down: bool, revoked: bool,
        foreign: bool, wrong_key: bool, corrupt: bool,
    }
    impl Fake {
        fn calls(&self) -> Vec<&'static str> { self.calls.lock().unwrap().clone() }
    }
    impl ResumePorts for Fake {
        fn load(&self, _: &CredentialScope)
            -> Result<Option<RefreshToken>, ResumeError>
        {
            self.calls.lock().unwrap().push("load_os");
            if self.store_down {
                return Err(ResumeError::SecretStore(SecretError::StoreFailure));
            }
            if self.missing { return Ok(None); }
            Ok(Some(RefreshToken::new("saved-refresh".into()).unwrap()))
        }
        fn refresh(&self, _: &str, _: &RefreshToken)
            -> Result<ShortLivedAccess, ResumeError>
        {
            self.calls.lock().unwrap().push("refresh_once");
            if self.revoked { return Err(ResumeError::RefreshRejected); }
            Ok(ShortLivedAccess {
                token: AccessToken::new("short-lived".into()).unwrap(),
                expires_in_seconds: 14_400,
            })
        }
        fn read_head(&self, _: &Scope, _: AccessToken, _: &TrustedBlake3)
            -> Result<VerifiedRead, ResumeError>
        {
            self.calls.lock().unwrap().push("account+head");
            if self.foreign {
                return Err(ResumeError::Remote(ReadError::AccountMismatch));
            }
            Ok(VerifiedRead {
                logical_key: if self.wrong_key {
                    ObjectKey::parse(&format!("blobs/{}", "a".repeat(64))).unwrap()
                } else { ObjectKey::head() },
                revision: DropboxRevision::parse("abcdef012345").unwrap(),
                bytes: if self.corrupt { b"malicious alternate HEAD".to_vec() }
                       else { HEAD.to_vec() },
            })
        }
    }
    fn run(fake: &Fake) -> Result<ReadOnlyHeadCandidate, ResumeError> {
        resume_with(fake, &scope(), "sync_app", &pin())
    }

    #[test]
    fn verified_restart_still_requires_publisher_approval() {
        let f = Fake::default();
        let candidate = run(&f).unwrap();
        assert_eq!(candidate.status(), ConnectionStatus::RemotePinnedNeedsPublisherApproval);
        assert_eq!(candidate.read_only_head_bytes(), HEAD);
        assert_eq!(f.calls(), vec!["load_os","refresh_once","account+head"]);
    }
    #[test]
    fn missing_or_locked_os_vault_has_no_network_fallback() {
        for f in [Fake { missing:true,..Default::default() },
                  Fake { store_down:true,..Default::default() }] {
            assert!(run(&f).is_err());
            assert_eq!(f.calls(), vec!["load_os"]);
        }
    }
    #[test]
    fn revoked_token_wrong_account_or_changed_head_fails_closed() {
        for f in [
            Fake { revoked:true,..Default::default() },
            Fake { foreign:true,..Default::default() },
            Fake { wrong_key:true,..Default::default() },
            Fake { corrupt:true,..Default::default() },
        ] {
            assert!(run(&f).is_err());
            assert!(!f.calls().is_empty());
        }
    }
    #[test]
    fn invalid_app_rejected_before_os_secret_read() {
        let f = Fake::default();
        assert!(matches!(resume_with(&f,&scope(),"../bad_app",&pin()),
            Err(ResumeError::InvalidOAuthApp)));
        assert!(f.calls().is_empty());
    }
    fn fake_response(status: u16, body:String) -> (String,thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut s,_) = l.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buf=Vec::new();
            let mut part=[0u8;4096];
            let end = loop {
                let n=s.read(&mut part).unwrap(); assert!(n>0);
                buf.extend_from_slice(&part[..n]);
                if let Some(i)=buf.windows(4).position(|x|x==b"\r\n\r\n") { break i+4; }
            };
            let headers = std::str::from_utf8(&buf[..end]).unwrap().to_owned();
            let size = headers.lines().find_map(|x|x.to_ascii_lowercase()
                .strip_prefix("content-length:").and_then(|s|s.trim().parse::<usize>().ok()))
                .unwrap_or(0);
            while buf.len()-end<size {
                let n=s.read(&mut part).unwrap();assert!(n>0);
                buf.extend_from_slice(&part[..n]);
            }
            let form=String::from_utf8(buf[end..end+size].to_vec()).unwrap();
            let reason=if status==200 {"OK"} else {"Bad Gateway"};
            write!(s,"HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            form
        });
        (format!("http://127.0.0.1:{port}/oauth2/token"),worker)
    }
    fn response() -> String {
        serde_json::json!({"access_token":"synthetic-access",
            "token_type":"bearer","expires_in":14400,"scope":READ_SCOPES}).to_string()
    }
    fn issue(body:String,status:u16)
        -> (Result<ShortLivedAccess,ResumeError>,HashMap<String,String>)
    {
        let (url,worker)=fake_response(status,body);
        let token=RefreshToken::new("saved-token".into()).unwrap();
        let outcome=refresh_at(&url,"sync_app",&token);
        let sent=worker.join().unwrap();
        (outcome,form_urlencoded::parse(sent.as_bytes()).into_owned().collect())
    }

    #[test]
    fn refresh_requests_exact_scope_and_never_sends_client_secret() {
        let (result,form)=issue(response(),200);
        assert_eq!(result.unwrap().expires_in_seconds,14400);
        assert_eq!(form["grant_type"],"refresh_token");
        assert_eq!(form["refresh_token"],"saved-token");
        assert_eq!(form["client_id"],"sync_app");
        assert_eq!(form["scope"],READ_SCOPES);
        assert!(!form.contains_key("client_secret"));
    }
    #[test]
    fn missing_or_escalated_scope_is_refused() {
        let mut missing:serde_json::Value=serde_json::from_str(&response()).unwrap();
        missing.as_object_mut().unwrap().remove("scope");
        assert!(matches!(issue(missing.to_string(),200).0,Err(ResumeError::MalformedResponse)));
        let malicious=response().replace("files.content.read","files.content.read files.content.write");
        assert!(matches!(issue(malicious,200).0,Err(ResumeError::ScopeEscalation)));
    }
    #[test]
    fn repeated_scope_cannot_replace_required_scope_and_order_is_irrelevant() {
        let mut duplicate: serde_json::Value =
            serde_json::from_str(&response()).unwrap();
        duplicate["scope"] = serde_json::json!(
            "account_info.read files.metadata.read files.metadata.read");
        assert!(matches!(issue(duplicate.to_string(), 200).0,
            Err(ResumeError::ScopeEscalation)));

        let mut reordered: serde_json::Value =
            serde_json::from_str(&response()).unwrap();
        reordered["scope"] = serde_json::json!(
            "files.content.read account_info.read files.metadata.read");
        assert!(issue(reordered.to_string(), 200).0.is_ok());
    }
    #[test]
    fn revoked_or_server_failure_never_retries_refresh() {
        assert!(matches!(issue(r#"{"error":"revoked"}"#.into(),401).0,
            Err(ResumeError::RefreshRejected)));
        assert!(matches!(issue(r#"{"error":"unknown"}"#.into(),503).0,
            Err(ResumeError::RefreshUncertain)));
    }
    #[test]
    fn malicious_rotated_refresh_token_response_is_rejected() {
        let mut payload:serde_json::Value=serde_json::from_str(&response()).unwrap();
        payload["refresh_token"]=serde_json::json!("new-secret");
        assert!(matches!(issue(payload.to_string(),200).0,
            Err(ResumeError::MalformedResponse)));
    }
    #[test]
    fn invalid_lifetime_is_rejected() {
        for n in [0,86401] {
            let mut payload:serde_json::Value=serde_json::from_str(&response()).unwrap();
            payload["expires_in"]=serde_json::json!(n);
            assert!(matches!(issue(payload.to_string(),200).0,
                Err(ResumeError::MalformedResponse)));
        }
    }
}
