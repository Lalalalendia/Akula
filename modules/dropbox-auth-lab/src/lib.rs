#![forbid(unsafe_code)]
//! Native PKCE with registered loopback callback; no embedded browser, persisted secrets, cloud writes or File Gate.

pub mod callback;
pub mod exchange;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fmt};
use subtle::ConstantTimeEq;
use url::{form_urlencoded, Url};
use zeroize::{Zeroize, Zeroizing};

const AUTHORIZE_ENDPOINT: &str = "https://www.dropbox.com/oauth2/authorize";
pub const TOKEN_ENDPOINT: &str = "https://api.dropboxapi.com/oauth2/token";
pub const READ_SCOPES: &str = "account_info.read files.metadata.read files.content.read";
const CALLBACK_PATH: &str = "/dropbox/callback";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    InvalidClientId, InvalidRedirect, RandomUnavailable, InvalidCallback,
    StateMismatch, Denied,
}

pub struct PendingPkce {
    client_id: String,
    redirect_uri: String,
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
}
impl fmt::Debug for PendingPkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingPkce(REDACTED)")
    }
}

pub struct AuthStart {
    /// For the system browser, not an embedded WebView.
    pub authorize_url: String,
    pending: PendingPkce,
}
impl fmt::Debug for AuthStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthStart(REDACTED)")
    }
}

/// Contains the authorization code and one-time verifier; never log or store.
pub struct TokenExchange {
    form_body: Zeroizing<String>,
}
impl fmt::Debug for TokenExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenExchange(REDACTED)")
    }
}
impl TokenExchange {
    /// Consumes one-time request data. Send once via HTTPS POST to TOKEN_ENDPOINT.
    pub fn into_form_body(self) -> Zeroizing<String> { self.form_body }
}

pub fn begin_readonly(client_id: &str, redirect_uri: &str) -> Result<AuthStart, AuthError> {
    if client_id.is_empty() || client_id.len() > 128
        || !client_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(AuthError::InvalidClientId);
    }
    let redirect = Url::parse(redirect_uri).map_err(|_| AuthError::InvalidRedirect)?;
    if redirect.scheme() != "http"
        || redirect.host_str() != Some("127.0.0.1")
        || redirect.port().is_none()
        || redirect.path() != CALLBACK_PATH
        || redirect.query().is_some()
        || redirect.fragment().is_some()
        || !redirect.username().is_empty()
        || redirect.password().is_some()
    {
        return Err(AuthError::InvalidRedirect);
    }
    let verifier = random_urlsafe()?;
    let state = random_urlsafe()?;
    let mut url = Url::parse(AUTHORIZE_ENDPOINT).expect("fixed OAuth URL");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("token_access_type", "offline")
        .append_pair("code_challenge", &code_challenge(&verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("redirect_uri", redirect.as_str())
        .append_pair("scope", READ_SCOPES)
        .append_pair("state", &state);
    Ok(AuthStart {
        authorize_url: url.into(),
        pending: PendingPkce {
            client_id: client_id.to_owned(),
            redirect_uri: redirect.as_str().to_owned(),
            state: Zeroizing::new(state),
            verifier: Zeroizing::new(verifier),
        },
    })
}

fn random_urlsafe() -> Result<String, AuthError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::RandomUnavailable)?;
    let result = URL_SAFE_NO_PAD.encode(bytes);
    bytes.zeroize();
    Ok(result)
}

fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

impl AuthStart {
    /// Consumes the session, including on denial or invalid callback.
    pub fn complete(self, callback_url: &str) -> Result<TokenExchange, AuthError> {
        self.pending.complete(callback_url)
    }
}

impl PendingPkce {
    fn complete(self, callback_url: &str) -> Result<TokenExchange, AuthError> {
        if callback_url.len() > 8192 { return Err(AuthError::InvalidCallback); }
        let callback = Url::parse(callback_url).map_err(|_| AuthError::InvalidCallback)?;
        let redirect = Url::parse(&self.redirect_uri).expect("validated redirect");
        if callback.scheme() != redirect.scheme()
            || callback.host_str() != redirect.host_str()
            || callback.port_or_known_default() != redirect.port_or_known_default()
            || callback.path() != redirect.path()
            || !callback.username().is_empty()
            || callback.password().is_some()
            || callback.fragment().is_some()
        {
            return Err(AuthError::InvalidCallback);
        }
        let (mut state, mut code, mut error) = (None, None, None);
        let mut seen = HashSet::new();
        for (k, v) in callback.query_pairs() {
            if !seen.insert(k.to_string()) { return Err(AuthError::InvalidCallback); }
            match k.as_ref() {
                "state" => state = Some(v.into_owned()),
                "code" => code = Some(v.into_owned()),
                "error" => error = Some(v.into_owned()),
                "error_description" => {}, // Do not expose provider-supplied text.
                _ => return Err(AuthError::InvalidCallback),
            }
        }
        let state = state.ok_or(AuthError::InvalidCallback)?;
        if state.len() != self.state.len()
            || !bool::from(state.as_bytes().ct_eq(self.state.as_bytes()))
        {
            return Err(AuthError::StateMismatch);
        }
        if error.is_some() {
            return if code.is_none() { Err(AuthError::Denied) }
                else { Err(AuthError::InvalidCallback) };
        }
        let code = Zeroizing::new(code.ok_or(AuthError::InvalidCallback)?);
        if code.is_empty() || code.len() > 4096
            || !code.bytes().all(|b| b.is_ascii_graphic() && b != b'&' && b != b'#')
        {
            return Err(AuthError::InvalidCallback);
        }
        let form = form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "authorization_code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("code_verifier", &self.verifier)
            .append_pair("code", &code)
            .finish();
        Ok(TokenExchange { form_body: Zeroizing::new(form) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const REDIRECT: &str = "http://127.0.0.1:47219/dropbox/callback";
    fn start() -> AuthStart { begin_readonly("app_key_123", REDIRECT).unwrap() }
    fn params(url: &str) -> std::collections::HashMap<String, String> {
        Url::parse(url).unwrap().query_pairs().into_owned().collect()
    }
    fn callback(state: &str) -> String {
        let mut url = Url::parse(REDIRECT).unwrap();
        url.query_pairs_mut().append_pair("state", state).append_pair("code", "code-A");
        url.into()
    }

    #[test]
    fn pkce_s256_matches_rfc7636_vector() {
        assert_eq!(code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
    #[test]
    fn start_uses_only_read_permissions_and_unique_secrets() {
        let a = start(); let b = start();
        let q = params(&a.authorize_url);
        let r = params(&b.authorize_url);
        assert!(a.authorize_url.starts_with(AUTHORIZE_ENDPOINT));
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["token_access_type"], "offline");
        assert_eq!(q["scope"], READ_SCOPES);
        assert!(!q["scope"].contains("write"));
        assert_eq!(q["redirect_uri"], REDIRECT);
        assert_eq!(q["state"].len(), 43);
        assert_eq!(q["code_challenge"].len(), 43);
        assert_ne!(q["state"], r["state"]);
        assert_ne!(q["code_challenge"], r["code_challenge"]);
        assert!(!a.authorize_url.contains("code_verifier"));
        assert_eq!(format!("{a:?}"), "AuthStart(REDACTED)");
    }
    #[test]
    fn correct_callback_yields_one_use_form_without_client_secret() {
        let a = start();
        let state = params(&a.authorize_url)["state"].clone();
        let token = a.complete(&callback(&state)).unwrap();
        assert_eq!(format!("{token:?}"), "TokenExchange(REDACTED)");
        let body = token.into_form_body();
        let form: std::collections::HashMap<String, String> =
            form_urlencoded::parse(body.as_bytes()).into_owned().collect();
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["client_id"], "app_key_123");
        assert_eq!(form["redirect_uri"], REDIRECT);
        assert_eq!(form["code"], "code-A");
        assert_eq!(form["code_verifier"].len(), 43);
        assert!(!body.contains("client_secret"));
    }
    #[test]
    fn wrong_state_and_duplicate_parameters_fail() {
        let a = start();
        assert!(matches!(a.complete(&callback(&"x".repeat(43))),
            Err(AuthError::StateMismatch)));
        let a = start();
        let state = params(&a.authorize_url)["state"].clone();
        assert!(matches!(a.complete(&format!("{REDIRECT}?state={state}&state={state}&code=x")),
            Err(AuthError::InvalidCallback)));
        let a = start();
        let state = params(&a.authorize_url)["state"].clone();
        assert!(matches!(a.complete(&format!("{REDIRECT}?state={state}&code=x&code=y")),
            Err(AuthError::InvalidCallback)));
    }
    #[test]
    fn wrong_origin_path_port_fragment_are_rejected() {
        for url in [
            "http://127.0.0.1:47220/dropbox/callback",
            "http://evil.example/dropbox/callback",
            "http://127.0.0.1:47219/other",
            "http://127.0.0.1:47219/dropbox/callback#token=fake",
        ] {
            let a = start();
            let state = params(&a.authorize_url)["state"].clone();
            let cb = format!("{url}?code=x&state={state}");
            assert!(matches!(a.complete(&cb), Err(AuthError::InvalidCallback)));
        }
    }
    #[test]
    fn denial_requires_correct_state() {
        let a = start();
        let state = params(&a.authorize_url)["state"].clone();
        assert!(matches!(a.complete(&format!("{REDIRECT}?error=access_denied&state={state}")),
            Err(AuthError::Denied)));
    }
    #[test]
    fn only_registered_loopback_style_redirects() {
        for uri in [
            "https://example.org/callback",
            "http://localhost:47219/dropbox/callback",
            "http://127.0.0.1/dropbox/callback",
            "http://127.0.0.1:47219/other",
            "http://user@127.0.0.1:47219/dropbox/callback",
            "http://127.0.0.1:47219/dropbox/callback?code=evil",
        ] {
            assert!(matches!(begin_readonly("app_key_123", uri),
                Err(AuthError::InvalidRedirect)));
        }
    }
}
