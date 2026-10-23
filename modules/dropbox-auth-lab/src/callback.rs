//! Single-use Dropbox PKCE callback on 127.0.0.1.
//! No browser automation, persisted token, file operations or sync authority.
//! Caller must register the exact returned URI in Dropbox App Console.

use crate::{begin_readonly, AuthError, AuthStart, TokenExchange};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const MAX_HEADER: usize = 8192;
const MAX_ATTEMPTS: usize = 12;
const CALLBACK_PATH: &str = "/dropbox/callback";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackError {
    PortUnavailable,
    InvalidRequest,
    Expired,
    TooManyRequests,
    Authorization(AuthError),
}

pub struct BoundLogin {
    listener: TcpListener,
    start: AuthStart,
    port: u16,
}

impl BoundLogin {
    /// Bind loopback BEFORE opening the system browser. The port must be
    /// registered for the OAuth application. Port 0 is for local tests only.
    pub fn bind(client_id: &str, port: u16) -> Result<Self, CallbackError> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|_| CallbackError::PortUnavailable)?;
        let port = listener.local_addr()
            .map_err(|_| CallbackError::PortUnavailable)?.port();
        let redirect = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let start = begin_readonly(client_id, &redirect)
            .map_err(CallbackError::Authorization)?;
        listener.set_nonblocking(true)
            .map_err(|_| CallbackError::PortUnavailable)?;
        Ok(Self { listener, start, port })
    }

    pub fn authorize_url(&self) -> &str { &self.start.authorize_url }
    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().expect("bound socket")
    }

    /// Consume the PKCE verifier on the first callback, valid or not.
    pub fn receive(self, max_wait: Duration) -> Result<TokenExchange, CallbackError> {
        let deadline = Instant::now() + max_wait.min(Duration::from_secs(120));
        let mut attempts = 0;
        while Instant::now() < deadline {
            match self.listener.accept() {
                Ok((mut stream, address)) => {
                    attempts += 1;
                    if attempts > MAX_ATTEMPTS { return Err(CallbackError::TooManyRequests); }
                    if !address.ip().is_loopback() {
                        return Err(CallbackError::InvalidRequest);
                    }
                    stream.set_read_timeout(Some(Duration::from_secs(2)))
                        .map_err(|_| CallbackError::InvalidRequest)?;
                    stream.set_write_timeout(Some(Duration::from_secs(2)))
                        .map_err(|_| CallbackError::InvalidRequest)?;
                    let target = match read_request(&mut stream, self.port) {
                        Ok(Some(target)) => target,
                        Ok(None) => {
                            reply(&mut stream, 404, "Not found");
                            continue;
                        }
                        Err(()) => {
                            reply(&mut stream, 400, "Invalid request");
                            return Err(CallbackError::InvalidRequest);
                        }
                    };
                    let target = Zeroizing::new(target);
                    let callback = Zeroizing::new(format!(
                        "http://127.0.0.1:{}{}", self.port, target,
                    ));
                    let result = self.start.complete(&callback);
                    match result {
                        Ok(token) => {
                            reply(&mut stream, 200, "Authorization completed. Close this tab.");
                            return Ok(token);
                        }
                        Err(reason) => {
                            reply(&mut stream, 400, "Authorization unsuccessful.");
                            return Err(CallbackError::Authorization(reason));
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(15));
                }
                Err(_) => return Err(CallbackError::InvalidRequest),
            }
        }
        Err(CallbackError::Expired)
    }
}

/// Only a relative GET with one valid Host is acceptable.
fn read_request(stream: &mut TcpStream, port: u16) -> Result<Option<String>, ()> {
    let mut buf = Zeroizing::new(Vec::with_capacity(1024));
    let mut chunk = [0u8; 1024];
    let end = loop {
        if buf.len() > MAX_HEADER { return Err(()); }
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        let n = stream.read(&mut chunk).map_err(|_| ())?;
        if n == 0 { return Err(()); }
        buf.extend_from_slice(&chunk[..n]);
    };
    if end > MAX_HEADER || buf.len() != end { return Err(()); }
    let raw = std::str::from_utf8(&buf).map_err(|_| ())?;
    let mut lines = raw.split("\r\n");
    let request = lines.next().ok_or(())?;
    let mut parts = request.split_whitespace();
    let (method, target, version) = (parts.next(), parts.next(), parts.next());
    if method != Some("GET") || version != Some("HTTP/1.1")
        || parts.next().is_some()
    { return Err(()); }
    let expected_host = format!("127.0.0.1:{port}");
    let mut hosts = lines.filter_map(|line| line.split_once(':')
        .filter(|(name, _)| name.eq_ignore_ascii_case("Host"))
        .map(|(_, host)| host.trim()));
    if hosts.next() != Some(expected_host.as_str()) || hosts.next().is_some() {
        return Err(());
    }
    let target = target.ok_or(())?;
    if !target.starts_with('/') || target.contains('#') || target.len() > MAX_HEADER {
        return Err(());
    }
    if target != CALLBACK_PATH && !target.starts_with(&format!("{CALLBACK_PATH}?")) {
        return Ok(None);
    }
    Ok(Some(target.to_owned()))
}

fn reply(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status { 200 => "OK", 404 => "Not Found", _ => "Bad Request" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nPragma: no-cache\r\n\
         Referrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n",
        body.len(),
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use url::{form_urlencoded, Url};

    fn state(login: &BoundLogin) -> String {
        Url::parse(login.authorize_url()).unwrap().query_pairs()
            .find(|(key, _)| key == "state").unwrap().1.into_owned()
    }
    fn send(port: u16, target: &str, host: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        write!(s, "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
        let mut response = String::new();
        s.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn single_use_callback_returns_code_without_exposing_it_in_browser() {
        let login = BoundLogin::bind("app_key_123", 0).unwrap();
        let port = login.local_addr().port();
        let st = state(&login);
        let worker = thread::spawn(move || login.receive(Duration::from_secs(3)));
        let response = send(port, &format!("{CALLBACK_PATH}?code=secret-code&state={st}"),
            &format!("127.0.0.1:{port}"));
        assert!(response.contains("200 OK"));
        assert!(response.contains("Cache-Control: no-store"));
        assert!(!response.contains("secret-code"));
        assert!(!response.contains(&st));
        let body = worker.join().unwrap().unwrap().into_form_body();
        let fields: HashMap<String, String> =
            form_urlencoded::parse(body.as_bytes()).into_owned().collect();
        assert_eq!(fields["code"], "secret-code");
        assert_eq!(fields["grant_type"], "authorization_code");
    }

    #[test]
    fn incorrect_host_and_state_fail_closed() {
        let login = BoundLogin::bind("app_key_123", 0).unwrap();
        let port = login.local_addr().port();
        let st = state(&login);
        let worker = thread::spawn(move || login.receive(Duration::from_secs(3)));
        let response = send(port, &format!("{CALLBACK_PATH}?code=code&state={st}"), "bad.example");
        assert!(response.contains("400 Bad Request"));
        assert!(matches!(worker.join().unwrap(), Err(CallbackError::InvalidRequest)));

        let login = BoundLogin::bind("app_key_123", 0).unwrap();
        let port = login.local_addr().port();
        let worker = thread::spawn(move || login.receive(Duration::from_secs(3)));
        let response = send(port, &format!("{CALLBACK_PATH}?code=code&state=wrong"),
            &format!("127.0.0.1:{port}"));
        assert!(response.contains("400 Bad Request"));
        assert!(matches!(worker.join().unwrap(),
            Err(CallbackError::Authorization(AuthError::StateMismatch))));
    }

    #[test]
    fn expiry_cannot_issue_a_token() {
        let login = BoundLogin::bind("app_key_123", 0).unwrap();
        assert!(matches!(login.receive(Duration::from_millis(35)),
            Err(CallbackError::Expired)));
    }
}
