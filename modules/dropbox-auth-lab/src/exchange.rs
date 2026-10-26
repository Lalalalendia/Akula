//! Single-use Dropbox OAuth token exchange. No token file or application
//! authoring. Public entrypoint pins the official HTTPS token endpoint.
//! Ambiguous responses are not retried: the authorization code is one use.

use crate::{TokenExchange, READ_SCOPES, TOKEN_ENDPOINT};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fmt,
    io::Read,
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_REPLY: usize = 65536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeError {
    TransportUncertain,
    Rejected,
    InvalidResponse,
    UnexpectedScope,
    OversizedResponse,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Seconds { Number(u64), Text(String) }
impl Seconds {
    fn parse(&self) -> Option<u64> {
        match self { Self::Number(n) => Some(*n), Self::Text(t) => t.parse().ok() }
    }
}

#[derive(Deserialize)]
struct WireTokens {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    account_id: String,
    scope: String,
    expires_in: Seconds,
}
impl Drop for WireTokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(ref mut refresh) = self.refresh_token { refresh.zeroize(); }
    }
}

pub struct ReadOnlyTokens {
    access: Zeroizing<String>,
    refresh: Zeroizing<String>,
    account_id: String,
    expires_in_seconds: u64,
}
impl fmt::Debug for ReadOnlyTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReadOnlyTokens(REDACTED)")
    }
}
impl ReadOnlyTokens {
    pub fn access_token(&self) -> &str { &self.access }
    pub fn refresh_token(&self) -> &str { &self.refresh }
    pub fn account_id(&self) -> &str { &self.account_id }
    pub fn expires_in_seconds(&self) -> u64 { self.expires_in_seconds }
}

/// Exchange one OAuth code at Dropbox. Never retry on timeout/5xx: the
/// server may have consumed the code before the response was lost.
pub fn exchange_once(exchange: TokenExchange) -> Result<ReadOnlyTokens, ExchangeError> {
    exchange_at(TOKEN_ENDPOINT, exchange)
}

/// Only tests can select a loopback endpoint; production is pinned.
fn exchange_at(endpoint: &str, exchange: TokenExchange)
    -> Result<ReadOnlyTokens, ExchangeError>
{
    let form = exchange.into_form_body();
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
        Ok(response) => response,
        Err(ureq::Error::Status(code, _)) if (400..500).contains(&code) =>
            return Err(ExchangeError::Rejected),
        Err(_) => return Err(ExchangeError::TransportUncertain),
    };
    if response.header("Content-Length")
        .and_then(|x| x.parse::<u64>().ok())
        .is_some_and(|n| n > MAX_REPLY as u64)
    {
        return Err(ExchangeError::OversizedResponse);
    }
    let mut raw = Zeroizing::new(Vec::new());
    response.into_reader().take((MAX_REPLY + 1) as u64)
        .read_to_end(&mut raw).map_err(|_| ExchangeError::TransportUncertain)?;
    if raw.len() > MAX_REPLY { return Err(ExchangeError::OversizedResponse); }
    let wire: WireTokens =
        serde_json::from_slice(&raw).map_err(|_| ExchangeError::InvalidResponse)?;
    if !wire.token_type.eq_ignore_ascii_case("bearer")
        || wire.access_token.is_empty() || wire.access_token.len() > 8192
        || !wire.access_token.bytes().all(|b| b.is_ascii_graphic())
        || wire.refresh_token.as_deref().is_none_or(|v| {
            v.is_empty() || v.len() > 8192 || !v.bytes().all(|b| b.is_ascii_graphic())
        })
        || !wire.account_id.starts_with("dbid:")
        || wire.account_id.len() > 256
        || !wire.account_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'-' || b == b'_')
    {
        return Err(ExchangeError::InvalidResponse);
    }
    let scopes: HashSet<&str> = wire.scope.split_whitespace().collect();
    let required: HashSet<&str> = READ_SCOPES.split_whitespace().collect();
    if scopes != required { return Err(ExchangeError::UnexpectedScope); }
    let seconds = wire.expires_in.parse()
        .filter(|&x| x > 0 && x <= 86400).ok_or(ExchangeError::InvalidResponse)?;
    Ok(ReadOnlyTokens {
        access: Zeroizing::new(wire.access_token.clone()),
        refresh: Zeroizing::new(wire.refresh_token.as_ref().unwrap().clone()),
        account_id: wire.account_id.clone(),
        expires_in_seconds: seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::begin_readonly;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    use url::{form_urlencoded, Url};

    fn valid_exchange() -> TokenExchange {
        let start = begin_readonly("app_key_123",
            "http://127.0.0.1:47219/dropbox/callback").unwrap();
        let state = Url::parse(&start.authorize_url).unwrap().query_pairs()
            .find(|(k, _)| k == "state").unwrap().1.into_owned();
        start.complete(&format!(
            "http://127.0.0.1:47219/dropbox/callback?state={state}&code=single-use-secret",
        )).unwrap()
    }

    fn fake_response(status: u16, body: String) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            let boundary;
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(p) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    boundary = p + 4;
                    break;
                }
            }
            let headers = std::str::from_utf8(&request[..boundary]).unwrap().to_owned();
            let length = headers.lines().find_map(|l| {
                l.to_ascii_lowercase().strip_prefix("content-length:")
                    .and_then(|x| x.trim().parse::<usize>().ok())
            }).unwrap();
            while request.len() - boundary < length {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
            }
            assert!(headers.starts_with("POST /oauth2/token HTTP/1.1"));
            assert!(headers.to_ascii_lowercase().contains("application/x-www-form-urlencoded"));
            let sent = String::from_utf8(request[boundary..boundary + length].to_vec()).unwrap();
            let reason = if status == 200 { "OK" } else { "Service Unavailable" };
            write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n\
                Content-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            sent
        });
        (format!("http://127.0.0.1:{port}/oauth2/token"), worker)
    }
    fn normal_body() -> String {
        serde_json::json!({
            "access_token": "fake-access", "refresh_token": "fake-refresh",
            "account_id": "dbid:alice", "scope": READ_SCOPES,
            "token_type": "bearer", "expires_in": 14400
        }).to_string()
    }

    #[test]
    fn valid_exchange_has_minimal_scope_and_redacted_debug() {
        let (url, worker) = fake_response(200, normal_body());
        let result = exchange_at(&url, valid_exchange()).unwrap();
        let sent = worker.join().unwrap();
        let parts: std::collections::HashMap<String,String> =
            form_urlencoded::parse(sent.as_bytes()).into_owned().collect();
        assert_eq!(parts["grant_type"], "authorization_code");
        assert_eq!(parts["client_id"], "app_key_123");
        assert_eq!(parts["code"], "single-use-secret");
        assert_eq!(parts["code_verifier"].len(), 43);
        assert!(!sent.contains("client_secret"));
        assert_eq!(result.access_token(), "fake-access");
        assert_eq!(result.refresh_token(), "fake-refresh");
        assert_eq!(result.account_id(), "dbid:alice");
        assert_eq!(result.expires_in_seconds(), 14400);
        assert_eq!(format!("{result:?}"), "ReadOnlyTokens(REDACTED)");
    }

    #[test]
    fn extra_permission_is_rejected_without_credential_acceptance() {
        let body = normal_body().replace(
            "files.content.read", "files.content.read files.content.write");
        let (url, worker) = fake_response(200, body);
        let result = exchange_at(&url, valid_exchange());
        worker.join().unwrap();
        assert!(matches!(result, Err(ExchangeError::UnexpectedScope)));
    }

    #[test]
    fn ambiguous_server_failure_never_automatically_reuses_code() {
        let (url, worker) = fake_response(503, r#"{"error":"unavailable"}"#.to_owned());
        assert!(matches!(exchange_at(&url, valid_exchange()),
            Err(ExchangeError::TransportUncertain)));
        let one_request = worker.join().unwrap();
        assert!(one_request.contains("single-use-secret"));
    }

    #[test]
    fn offline_refresh_token_is_required() {
        let mut payload: serde_json::Value = serde_json::from_str(&normal_body()).unwrap();
        payload.as_object_mut().unwrap().remove("refresh_token");
        let (url, worker) = fake_response(200, payload.to_string());
        assert!(matches!(exchange_at(&url, valid_exchange()),
            Err(ExchangeError::InvalidResponse)));
        worker.join().unwrap();
    }
}
