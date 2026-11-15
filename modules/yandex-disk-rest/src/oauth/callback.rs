//! Single-use PKCE callback bound to the registered Yandex native-app URI.
//! Never print OAuth codes/state or put them in settings, files or logs.
//! No user login automation, remote writes, or Sync File Gate authority.

use super::{YandexOAuthError, YandexPkceStart, YandexTokenExchange};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const CALLBACK_PATH: &str = "/akula/yandex/callback";
const MAX_HEADER: usize = 8192;
const MAX_ATTEMPTS: usize = 12;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CallbackError {
    PortUnavailable,
    InvalidRequest,
    Timeout,
    TooManyRequests,
    Authorization(YandexOAuthError),
}

/// Strict loopback server. Construct before opening the system browser.
/// The port must belong to the Yandex OAuth app's registered Redirect URI.
pub struct BoundLogin {
    listener: TcpListener,
    auth: YandexPkceStart,
    port: u16,
}

impl BoundLogin {
    pub fn bind(client_id: &str, registered_port: u16) -> Result<Self, CallbackError> {
        if registered_port == 0 {
            return Err(CallbackError::PortUnavailable);
        }
        Self::bind_inner(client_id, registered_port)
    }

    fn bind_inner(client_id: &str, port: u16) -> Result<Self, CallbackError> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|_| CallbackError::PortUnavailable)?;
        let actual_port = listener.local_addr()
            .map_err(|_| CallbackError::PortUnavailable)?
            .port();
        let redirect = format!("http://127.0.0.1:{actual_port}{CALLBACK_PATH}");
        let auth = YandexPkceStart::begin(client_id, &redirect)
            .map_err(CallbackError::Authorization)?;
        listener.set_nonblocking(true)
            .map_err(|_| CallbackError::PortUnavailable)?;
        Ok(Self { listener, auth, port: actual_port })
    }

    /// Open only with an explicit user action in the default system browser.
    pub fn browser_authorize_url(&self) -> &str {
        &self.auth.authorization_url
    }

    /// Must run on a worker, not the app's UI thread. Consumes the attempt.
    pub fn receive(self, max_wait: Duration) -> Result<YandexTokenExchange, CallbackError> {
        let deadline = Instant::now() + max_wait.min(Duration::from_secs(120));
        let mut attempts = 0;
        while Instant::now() < deadline {
            match self.listener.accept() {
                Ok((mut socket, addr)) => {
                    attempts += 1;
                    if attempts > MAX_ATTEMPTS {
                        return Err(CallbackError::TooManyRequests);
                    }
                    if !addr.ip().is_loopback() {
                        return Err(CallbackError::InvalidRequest);
                    }
                    socket.set_read_timeout(Some(SOCKET_TIMEOUT))
                        .map_err(|_| CallbackError::InvalidRequest)?;
                    socket.set_write_timeout(Some(SOCKET_TIMEOUT))
                        .map_err(|_| CallbackError::InvalidRequest)?;
                    let target = match read_request(&mut socket, self.port) {
                        Ok(Some(target)) => target,
                        Ok(None) => {
                            reply(&mut socket, 404, "Not found.");
                            continue;
                        }
                        Err(_) => {
                            reply(&mut socket, 400, "Invalid callback.");
                            return Err(CallbackError::InvalidRequest);
                        }
                    };
                    let callback = Zeroizing::new(format!(
                        "http://127.0.0.1:{}{}", self.port, target.as_str()
                    ));
                    return match self.auth.pending.complete(callback.as_str()) {
                        Ok(exchange) => {
                            reply(&mut socket, 200, "Authorization received. Close this tab.");
                            Ok(exchange)
                        }
                        Err(error) => {
                            reply(&mut socket, 400, "Authorization rejected.");
                            Err(CallbackError::Authorization(error))
                        }
                    };
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(15));
                }
                Err(_) => return Err(CallbackError::InvalidRequest),
            }
        }
        Err(CallbackError::Timeout)
    }

    #[cfg(test)]
    fn test_bind() -> Self {
        Self::bind_inner("4760187d81bc4b7799476b42b5103713", 0).unwrap()
    }

    #[cfg(test)]
    fn port(&self) -> u16 {
        self.port
    }
}

/// Bound HTTP/1.1 parser. Disallows bodies, duplicate Host, absolute-form
/// requests, any foreign path and response reflection of callback secrets.
fn read_request(
    stream: &mut TcpStream,
    port: u16,
) -> Result<Option<Zeroizing<String>>, CallbackError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(1024));
    let mut chunk = Zeroizing::new([0u8; 1024]);
    let header_end = loop {
        if bytes.len() >= MAX_HEADER {
            return Err(CallbackError::InvalidRequest);
        }
        if let Some(start) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break start + 4;
        }
        let read = stream.read(&mut *chunk).map_err(|_| CallbackError::InvalidRequest)?;
        if read == 0 {
            return Err(CallbackError::InvalidRequest);
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    if header_end > MAX_HEADER || bytes.len() != header_end {
        return Err(CallbackError::InvalidRequest);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| CallbackError::InvalidRequest)?;
    let mut lines = text.split("\r\n");
    let request = lines.next().ok_or(CallbackError::InvalidRequest)?;
    if request.contains('\t') {
        return Err(CallbackError::InvalidRequest);
    }
    let mut parts = request.split(' ');
    if parts.next() != Some("GET") {
        return Err(CallbackError::InvalidRequest);
    }
    let target = parts.next().ok_or(CallbackError::InvalidRequest)?;
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some()
        || !target.is_ascii() || !target.starts_with('/') || target.contains('#')
    {
        return Err(CallbackError::InvalidRequest);
    }

    let mut found_host = false;
    let expected_host = format!("127.0.0.1:{port}");
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(CallbackError::InvalidRequest)?;
        if name.is_empty()
            || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(CallbackError::InvalidRequest);
        }
        if name.eq_ignore_ascii_case("Host") {
            if found_host || value.trim() != expected_host {
                return Err(CallbackError::InvalidRequest);
            }
            found_host = true;
        }
        if name.eq_ignore_ascii_case("Content-Length")
            || name.eq_ignore_ascii_case("Transfer-Encoding")
        {
            return Err(CallbackError::InvalidRequest);
        }
    }
    if !found_host {
        return Err(CallbackError::InvalidRequest);
    }
    if target != CALLBACK_PATH && !target.starts_with(&format!("{CALLBACK_PATH}?")) {
        return Ok(None);
    }
    Ok(Some(Zeroizing::new(target.to_owned())))
}

fn reply(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nPragma: no-cache\r\n\
         Referrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n",
        body.len(),
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use url::{form_urlencoded, Url};

    fn send(port: u16, target: &str, host: &str, extra: &str) -> String {
        let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket.set_read_timeout(Some(SOCKET_TIMEOUT)).unwrap();
        write!(
            socket,
            "GET {target} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n"
        ).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    }

    fn csrf(login: &BoundLogin) -> String {
        Url::parse(login.browser_authorize_url())
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned()
    }

    #[test]
    fn exact_callback_is_once_only_and_never_reflects_secrets() {
        let login = BoundLogin::test_bind();
        let port = login.port();
        let state = csrf(&login);
        let auth = Url::parse(login.browser_authorize_url()).unwrap();
        let params: BTreeMap<_, _> = auth.query_pairs().into_owned().collect();
        assert_eq!(params["redirect_uri"], format!("http://127.0.0.1:{port}{CALLBACK_PATH}"));
        assert_eq!(params["code_challenge_method"], "S256");
        let worker = thread::spawn(move || login.receive(Duration::from_secs(2)));
        let response = send(port, &format!("{CALLBACK_PATH}?code=onceCode&state={state}"),
            &format!("127.0.0.1:{port}"), "");
        assert!(response.contains("200 OK"));
        assert!(response.contains("Cache-Control: no-store"));
        assert!(!response.contains(&state));
        assert!(!response.contains("onceCode"));
        let exchange = worker.join().unwrap().unwrap();
        let body = exchange.form_body();
        let fields: BTreeMap<_, _> = form_urlencoded::parse(body.as_bytes()).into_owned().collect();
        assert_eq!(fields["code"], "onceCode");
        assert_eq!(fields["grant_type"], "authorization_code");
    }

    #[test]
    fn foreign_host_and_forged_csrf_never_complete_login() {
        let login = BoundLogin::test_bind();
        let port = login.port();
        let state = csrf(&login);
        let worker = thread::spawn(move || login.receive(Duration::from_secs(2)));
        let answer = send(port, &format!("{CALLBACK_PATH}?code=a&state={state}"),
            "foreign.example", "");
        assert!(answer.contains("400 Bad Request"));
        assert!(matches!(worker.join().unwrap(), Err(CallbackError::InvalidRequest)));

        let login = BoundLogin::test_bind();
        let port = login.port();
        let worker = thread::spawn(move || login.receive(Duration::from_secs(2)));
        let answer = send(port, &format!("{CALLBACK_PATH}?code=a&state=wrong"),
            &format!("127.0.0.1:{port}"), "");
        assert!(answer.contains("400 Bad Request"));
        assert!(matches!(
            worker.join().unwrap(),
            Err(CallbackError::Authorization(YandexOAuthError::CallbackWrongState))
        ));
    }

    #[test]
    fn duplicate_host_body_and_automatic_port_are_rejected() {
        assert!(matches!(
            BoundLogin::bind("4760187d81bc4b7799476b42b5103713", 0),
            Err(CallbackError::PortUnavailable)
        ));
        for extra in ["Host: 127.0.0.1:99\r\n", "Content-Length: 4\r\n"] {
            let login = BoundLogin::test_bind();
            let port = login.port();
            let state = csrf(&login);
            let worker = thread::spawn(move || login.receive(Duration::from_secs(2)));
            let response = send(port, &format!("{CALLBACK_PATH}?code=a&state={state}"),
                &format!("127.0.0.1:{port}"), extra);
            assert!(response.contains("400 Bad Request"));
            assert!(matches!(worker.join().unwrap(), Err(CallbackError::InvalidRequest)));
        }
    }

    #[test]
    fn unrelated_path_returns_404_without_consuming_one_use_state() {
        let login = BoundLogin::test_bind();
        let port = login.port();
        let state = csrf(&login);
        let worker = thread::spawn(move || login.receive(Duration::from_secs(2)));
        assert!(send(port, "/favicon.ico", &format!("127.0.0.1:{port}"), "")
            .contains("404 Not Found"));
        assert!(send(port, &format!("{CALLBACK_PATH}?code=valid&state={state}"),
            &format!("127.0.0.1:{port}"), "").contains("200 OK"));
        assert_eq!(
            form_urlencoded::parse(worker.join().unwrap().unwrap().form_body().as_bytes())
                .find(|(k, _)| k == "code").unwrap().1,
            "valid"
        );
    }

    #[test]
    fn bounded_wait_expires_without_issuing_authorization() {
        let login = BoundLogin::test_bind();
        assert!(matches!(
            login.receive(Duration::from_millis(35)), Err(CallbackError::Timeout)
        ));
    }
}
