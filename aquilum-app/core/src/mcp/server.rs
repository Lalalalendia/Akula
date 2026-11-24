use super::agent_principals::{AgentCredentialStore, AuthenticatedAgent};
use super::http::{refused, Connection, Failure, Head, Response};
use super::protocol;
#[cfg(test)]
use super::agent_scoped_read::HostPinnedReadRoot;
#[cfg(test)]
use super::agent_vault_grants::{AgentVaultGrantStore, HostVaultRegistration};
#[cfg(test)]
use crate::plugins::VaultId;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;
#[cfg(test)]
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
mod browser_guard;
const MAX_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 16;
const MAX_PARALLEL_READS: usize = 4;
// Bound how long the accept loop waits for a reclaimed *idle* connection.
const RECLAIM_WAIT: Duration = Duration::from_millis(500);

pub type SharedToolCaller = Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

pub struct RunningServer {
    address: SocketAddr,
    endpoint: Arc<Endpoint>,
    accepting: JoinHandle<()>,
}

/// Never treat host-issued agent credentials as the legacy full-access bearer.
/// ScopedDenyAll is an opt-in bootstrap until per-vault grants exist.
enum AuthenticationMode {
    Legacy(String),
    ScopedDenyAll(Arc<Mutex<AgentCredentialStore>>),
    #[cfg(test)]
    ScopedReadFixture(Arc<ScopedReadFixture>),
}

/// This test-only origin can NEVER be created by an installed product.
#[cfg(test)]
struct ScopedReadFixture {
    credentials: Arc<Mutex<AgentCredentialStore>>,
    grants: Arc<Mutex<AgentVaultGrantStore>>,
    roots: BTreeMap<VaultId, (HostVaultRegistration, HostPinnedReadRoot)>,
}

/// Host-owned identity, never created from MCP clientInfo or tool arguments.
/// No Agent variant has data capabilities at this bootstrap stage.
enum RequestPrincipal {
    Legacy,
    Agent(AuthenticatedAgent),
}

impl AuthenticationMode {
    fn authenticate(&self, head: &Head) -> Option<RequestPrincipal> {
        match self {
            Self::Legacy(token) => authorized(head, token).then_some(RequestPrincipal::Legacy),
            Self::ScopedDenyAll(store) => authenticate_agent(store, head),
            #[cfg(test)]
            Self::ScopedReadFixture(fixture) => authenticate_agent(&fixture.credentials, head),
        }
    }
}

fn authenticate_agent(
    store: &Arc<Mutex<AgentCredentialStore>>,
    head: &Head,
) -> Option<RequestPrincipal> {
    if head.header_count("authorization") != 1 {
        return None;
    }
    let (scheme, credential) = head.header("authorization")?.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") || credential.trim().is_empty() {
        return None;
    }
    let store = store.lock().ok()?;
    store.authenticate(credential.trim()).ok().map(RequestPrincipal::Agent)
}

struct Endpoint {
    authentication: AuthenticationMode,
    listen_port: u16,
    call: SharedToolCaller,
    stopping: AtomicBool,
    turn: DispatchGate,
    open: Mutex<ConnectionSlots>,
    freed: Condvar,
}

#[derive(Default)]
struct ConnectionSlots {
    count: usize,
    next_id: u64,
    idle: Vec<IdleConnection>,
}

struct IdleConnection {
    id: u64,
    evicted: Arc<AtomicBool>,
}

struct Slot {
    endpoint: Arc<Endpoint>,
    id: u64,
    evicted: Arc<AtomicBool>,
}

enum Capacity {
    Stopping,
    Saturated,
}

// Admission is bounded independently of TCP keep-alive connections.
// Pending writers block *new* readers to prevent read-heavy agent traffic
// from starving updates. Every request retains its permit through the reply,
// preserving the existing stop/drain and per-connection response contracts.
#[derive(Default)]
struct DispatchState {
    readers: usize,
    writer: bool,
    waiting_writers: usize,
    next_read_ticket: u64,
    serving_read_ticket: u64,
    next_write_ticket: u64,
    serving_write_ticket: u64,
}

struct DispatchGate {
    state: Mutex<DispatchState>,
    ready: Condvar,
}

struct DispatchPermit<'a> {
    gate: &'a DispatchGate,
    read: bool,
}

impl DispatchGate {
    fn new() -> Self {
        Self {
            state: Mutex::new(DispatchState::default()),
            ready: Condvar::new(),
        }
    }

    fn acquire(&self, read: bool, stopping: &AtomicBool) -> Option<DispatchPermit<'_>> {
        let mut state = lock(&self.state);
        let ticket = if read {
            let ticket = state.next_read_ticket;
            state.next_read_ticket = state.next_read_ticket.wrapping_add(1);
            ticket
        } else {
            let ticket = state.next_write_ticket;
            state.next_write_ticket = state.next_write_ticket.wrapping_add(1);
            state.waiting_writers += 1;
            ticket
        };
        loop {
            let blocked = if read {
                state.writer
                    || state.waiting_writers != 0
                    || state.readers >= MAX_PARALLEL_READS
                    || ticket != state.serving_read_ticket
            } else {
                state.writer || state.readers != 0 || ticket != state.serving_write_ticket
            };
            if !blocked || stopping.load(Ordering::SeqCst) {
                break;
            }
            state = self.ready.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        if !read {
            state.waiting_writers -= 1;
        }
        if stopping.load(Ordering::SeqCst) {
            self.ready.notify_all();
            return None;
        }
        if read {
            state.readers += 1;
            state.serving_read_ticket = state.serving_read_ticket.wrapping_add(1);
            self.ready.notify_all();
        } else {
            state.writer = true;
        }
        Some(DispatchPermit { gate: self, read })
    }

    fn wake(&self) {
        let _state = lock(&self.state);
        self.ready.notify_all();
    }

    fn drain(&self) {
        let mut state = lock(&self.state);
        while state.writer || state.readers != 0 {
            state = self.ready.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Drop for DispatchPermit<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.gate.state);
        if self.read {
            state.readers -= 1;
        } else {
            state.writer = false;
            state.serving_write_ticket = state.serving_write_ticket.wrapping_add(1);
        }
        self.gate.ready.notify_all();
    }
}

impl Slot {
    fn acquire(endpoint: &Arc<Endpoint>) -> Result<Self, Capacity> {
        let mut slots = lock(&endpoint.open);
        if endpoint.stopping.load(Ordering::SeqCst) {
            return Err(Capacity::Stopping);
        }
        if slots.count >= MAX_CONNECTIONS {
            // A completed HTTP reply is idle and has no tool mutation in
            // progress. Reclaim only that socket; never interrupt active
            // headers, request bodies, tool calls or response writes.
            // Prefer the oldest completed keep-alive. Evicting the newest
            // risks closing the socket before its previous response has
            // even been observed by the client.
            if !slots.idle.is_empty() {
                let idle = slots.idle.remove(0);
                // Portable cooperative wakeup. On Windows, shutdown on a
                // cloned socket does not reliably wake another recv thread.
                // The idle header reader polls the eviction bit every 50 ms.
                idle.evicted.store(true, Ordering::SeqCst);
                let (next, _) = endpoint
                    .freed
                    .wait_timeout_while(slots, RECLAIM_WAIT, |slots| {
                        slots.count >= MAX_CONNECTIONS
                            && !endpoint.stopping.load(Ordering::SeqCst)
                    })
                    .unwrap_or_else(PoisonError::into_inner);
                slots = next;
            }
        }
        if endpoint.stopping.load(Ordering::SeqCst) {
            return Err(Capacity::Stopping);
        }
        if slots.count >= MAX_CONNECTIONS {
            return Err(Capacity::Saturated);
        }
        let id = slots.next_id;
        slots.next_id = slots.next_id.wrapping_add(1);
        slots.count += 1;
        Ok(Self {
            endpoint: Arc::clone(endpoint),
            id,
            evicted: Arc::new(AtomicBool::new(false)),
        })
    }

    fn idle(&self) {
        // Register only between complete replies and the next request head.
        // No duplicated kernel socket or forced TCP reset is necessary.
        let mut slots = lock(&self.endpoint.open);
        if !self.endpoint.stopping.load(Ordering::SeqCst)
            && !self.evicted.load(Ordering::SeqCst)
        {
            slots.idle.push(IdleConnection {
                id: self.id,
                evicted: Arc::clone(&self.evicted),
            });
        }
    }

    fn activate(&self) -> bool {
        let mut slots = lock(&self.endpoint.open);
        slots.idle.retain(|connection| connection.id != self.id);
        !self.evicted.load(Ordering::SeqCst)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut slots = lock(&self.endpoint.open);
        slots.idle.retain(|connection| connection.id != self.id);
        slots.count -= 1;
        self.endpoint.freed.notify_all();
    }
}

impl RunningServer {
    pub fn start(port: u16, token: String, call: SharedToolCaller) -> Result<Self, String> {
        Self::start_with_auth(port, AuthenticationMode::Legacy(token), call)
    }

    /// Host-only opt-in admission with no tools. In particular, do not pass
    /// an unrestricted tool caller after agent credential validation.
    pub(crate) fn start_scoped_deny_all(
        port: u16,
        store: Arc<Mutex<AgentCredentialStore>>,
    ) -> Result<Self, String> {
        let deny: SharedToolCaller = Arc::new(|_name, _args| {
            Err("MCP_AGENT_SCOPE_DENIED".to_owned())
        });
        Self::start_with_auth(port, AuthenticationMode::ScopedDenyAll(store), deny)
    }

    /// A test-only positive MCP listener; NEVER compiled in shipped Akula.
    #[cfg(test)]
    fn start_scoped_read_fixture(
        port: u16,
        fixture: ScopedReadFixture,
    ) -> Result<Self, String> {
        let deny: SharedToolCaller =
            Arc::new(|_, _| Err("MCP_AGENT_SCOPE_DENIED".to_owned()));
        Self::start_with_auth(
            port,
            AuthenticationMode::ScopedReadFixture(Arc::new(fixture)),
            deny,
        )
    }

    fn start_with_auth(
        port: u16,
        authentication: AuthenticationMode,
        call: SharedToolCaller,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let endpoint = Arc::new(Endpoint {
            authentication,
            listen_port: address.port(),
            call,
            stopping: AtomicBool::new(false),
            turn: DispatchGate::new(),
            open: Mutex::new(ConnectionSlots::default()),
            freed: Condvar::new(),
        });
        let accepted = Arc::clone(&endpoint);
        let accepting = thread::Builder::new()
            .name("aquilum-mcp".to_owned())
            .spawn(move || accept(&listener, &accepted))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            address,
            endpoint,
            accepting,
        })
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    pub fn stop(self) {
        self.endpoint.stopping.store(true, Ordering::SeqCst);
        self.endpoint.turn.wake();
        drop(lock(&self.endpoint.open));
        self.endpoint.freed.notify_all();
        let _ = TcpStream::connect(self.address);
        let _ = self.accepting.join();
        self.endpoint.turn.drain();
    }
}

impl Endpoint {
    fn answer(
        &self,
        body: &str,
        principal: RequestPrincipal,
        _head: &Head,
        connection: &mut Connection,
        keep_alive: bool,
    ) -> Result<bool, Failure> {
        let read = protocol::allows_parallel_read(body);
        let _turn = self
            .turn
            .acquire(read, &self.stopping)
            .ok_or_else(|| refused(503, "MCP-сервер остановлен"))?;
        if self.stopping.load(Ordering::SeqCst) {
            return Err(refused(503, "MCP-сервер остановлен"));
        }
        let output = match (principal, &self.authentication) {
            (RequestPrincipal::Legacy, _) => {
                protocol::handle_message(body, self.call.as_ref())
            }
            #[cfg(test)]
            (RequestPrincipal::Agent(agent), AuthenticationMode::ScopedReadFixture(fixture)) => {
                // Tool discovery and every tool invocation independently
                // check the currently authenticated principal and grant.
                let eligible = fixture.roots.values().any(|(receipt, _)| {
                    lock(&fixture.grants)
                        .authorize_notes_read(agent, *receipt)
                        .is_ok()
                });
                if !eligible {
                    protocol::handle_without_tools(body)
                } else {
                    let bearer = _head
                        .header("authorization")
                        .and_then(|value| value.trim().split_once(' '))
                        .map(|(_, token)| token.trim())
                        .unwrap_or("");
                    let dispatch = |name: &str, args: &Value| -> Result<Value, String> {
                        let denied = || "MCP_AGENT_SCOPE_DENIED".to_owned();
                        if name != "read_note" {
                            return Err(denied());
                        }
                        let requested = args
                            .get("vaultId")
                            .and_then(Value::as_str)
                            .and_then(|v| uuid::Uuid::parse_str(v).ok())
                            .ok_or_else(denied)?;
                        let path = args
                            .get("note")
                            .and_then(Value::as_str)
                            .ok_or_else(denied)?;
                        // A guest UUID only selects an ALREADY trusted test
                        // root: it never constructs an authority/registration.
                        let (vault, (_, root)) = fixture
                            .roots
                            .iter()
                            .find(|(vault, _)| vault.as_uuid() == requested)
                            .ok_or_else(denied)?;
                        let credentials = lock(&fixture.credentials);
                        let grants = lock(&fixture.grants);
                        let checked = root
                            .read_checked(&credentials, &grants, bearer, *vault, path)
                            .map_err(|_| denied())?;
                        Ok(json!({
                            "note": checked.path,
                            "content": checked.content,
                            "revision": checked.revision,
                        }))
                    };
                    protocol::handle_fixture_scoped_read(body, &dispatch)
                }
            }
            (RequestPrincipal::Agent(_), _) => protocol::handle_without_tools(body),
        };
        let response = match output {
            Some(payload) => Response::json(payload),
            None => Response::text(202, ""),
        };
        connection.send(&response, keep_alive)?;
        Ok(keep_alive)
    }
}

fn accept(listener: &TcpListener, endpoint: &Arc<Endpoint>) {
    for stream in listener.incoming() {
        if endpoint.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else { continue };
        let slot = match Slot::acquire(endpoint) {
            Ok(slot) => slot,
            Err(Capacity::Stopping) => return,
            Err(Capacity::Saturated) => {
                // Explicit bounded backpressure instead of holding the accept
                // loop behind 16 occupied sockets for up to 60 seconds.
                if let Ok(mut connection) = Connection::new(&stream) {
                    let _ = stream.set_write_timeout(Some(RECLAIM_WAIT));
                    let _ = connection.send(
                        &Response::text(503, "MCP: все соединения заняты, повторите запрос"),
                        false,
                    );
                }
                continue;
            }
        };
        let _ = thread::Builder::new()
            .name("aquilum-mcp-connection".to_owned())
            .spawn(move || serve(&stream, &slot));
    }
}

fn serve(stream: &TcpStream, slot: &Slot) {
    let Ok(mut connection) = Connection::new(stream) else {
        return;
    };
    let closing = loop {
        match exchange(&mut connection, &slot.endpoint, slot) {
            Ok(true) => {
                if !connection.has_buffered_data() {
                    slot.idle();
                }
            }
            Ok(false) => break Ok(()),
            Err(Failure::Refused(response)) => break connection.send(&response, false),
            Err(Failure::Gone) => return,
        }
    };
    if closing.is_ok() {
        connection.hang_up();
    }
}

fn exchange(connection: &mut Connection, endpoint: &Endpoint, slot: &Slot) -> Result<bool, Failure> {
    let head = connection.read_head(|received| {
        if received {
            slot.activate()
        } else {
            !slot.evicted.load(Ordering::SeqCst)
        }
    })?;
    if !slot.activate() {
        return Err(Failure::Gone);
    }
    let principal = admit(&head, &endpoint.authentication, endpoint.listen_port)?;
    connection.mark_authenticated();
    let body = connection.read_body(&head, MAX_BODY_BYTES)?;
    let body = String::from_utf8(body).map_err(|_| refused(400, "Тело запроса не в UTF-8"))?;
    endpoint.answer(&body, principal, &head, connection, head.keeps_alive())
}
fn admit(
    head: &Head,
    authentication: &AuthenticationMode,
    listen_port: u16,
) -> Result<RequestPrincipal, Failure> {
    if head.method != "POST" {
        let response = Response::text(405, "Поддерживается только POST").allowing("POST");
        return Err(Failure::Refused(response));
    }
    // Loopback binding and Origin checks do not protect against DNS rebinding
    // when an attacker-controlled hostname resolves to 127.0.0.1. Require
    // exactly one literal loopback Host before processing authentication or
    // any MCP tool. The TCP port (if specified) must match this listener.
    if head.header_count("host") != 1
        || head.header("host").is_none_or(|value| value.trim().is_empty())
    {
        return Err(refused(400, "Отсутствует или дублируется Host"));
    }
    if !head
        .header("host")
        .is_some_and(|value| is_loopback_host(value, listen_port))
    {
        return Err(refused(403, "Запрещённый Host"));
    }
    browser_guard::validate(head)?;
    let principal = authentication.authenticate(head)
        .ok_or_else(|| refused(401, "Неверный токен"))?;
    // The legacy HTTP endpoint supports only 2025-06-18. An explicit
    // modern/unknown header cannot silently enable another wire protocol.
    // Missing headers remain a documented legacy-bridge compatibility case.
    if head.header_count("mcp-protocol-version") > 1
        || head
            .header("mcp-protocol-version")
            .is_some_and(|value| value.trim() != protocol::PROTOCOL_VERSION)
    {
        return Err(refused(400, "Неподдерживаемая версия MCP-Protocol-Version"));
    }
    Ok(principal)
}

// The listener is fixed to IPv4 loopback. A literal loopback Host
// allowlist avoids hostname resolution and suffix-trick bypasses. Permit
// absent port for older native HTTP clients, but never a different port.
fn is_loopback_host(value: &str, listen_port: u16) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let suffix = ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .find_map(|host| {
            value
                .strip_prefix(*host)
                .filter(|suffix| suffix.is_empty() || suffix.starts_with(':'))
        });
    let Some(suffix) = suffix else {
        return false;
    };
    if suffix.is_empty() {
        return true;
    }
    let digits = &suffix[1..];
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && digits.parse::<u16>().ok() == Some(listen_port)
}

fn is_loopback_origin(origin: &str) -> bool {
    let value = origin.trim().to_ascii_lowercase();
    let Some(authority) = value.strip_prefix("http://") else {
        return false;
    };
    if authority.contains('/') || authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((host, port)) if port.is_empty() || is_port(port.trim_start_matches(':')) => host,
            _ => return false,
        },
        None => match authority.split_once(':') {
            Some((host, port)) if is_port(port) => host,
            Some(_) => return false,
            None => authority,
        },
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn is_port(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn authorized(head: &Head, token: &str) -> bool {
    head.header("authorization")
        .and_then(|value| value.trim().split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .is_some_and(|(_, credentials)| equal_in_constant_time(credentials.trim(), token))
}

fn equal_in_constant_time(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{self, Read, Write};
    use std::net::Shutdown;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    const TOKEN: &str = "test-token-123";

    fn start(call: SharedToolCaller) -> (RunningServer, String) {
        let server = RunningServer::start(0, TOKEN.to_owned(), call).unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", server.port());
        (server, url)
    }

    fn server() -> (RunningServer, String) {
        start(Arc::new(|name: &str, _arguments: &Value| {
            Ok(json!({ "echo": name }))
        }))
    }

    fn slow_tool(pause: Duration) -> SharedToolCaller {
        Arc::new(move |_name: &str, _arguments: &Value| {
            thread::sleep(pause);
            Ok(Value::Null)
        })
    }

    fn initialize() -> String {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": protocol::PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "Akula HTTP test", "version": "1.0" }
            }
        })
        .to_string()
    }

    fn tool_call(name: &str) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": name, "arguments": {} },
        })
        .to_string()
    }

    fn bearer() -> String {
        format!("Bearer {TOKEN}")
    }

    fn status(outcome: Result<ureq::Response, ureq::Error>) -> Option<u16> {
        match outcome {
            Ok(response) => Some(response.status()),
            Err(ureq::Error::Status(status, _)) => Some(status),
            Err(_) => None,
        }
    }

    fn head(extra: &str) -> String {
        format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TOKEN}\r\n{extra}\r\n")
    }

    fn sized(body: &str, extra: &str) -> String {
        let head = head(&format!("Content-Length: {}\r\n{extra}", body.len()));
        format!("{head}{body}")
    }

    struct Answer {
        status: u16,
        connection: String,
        body: String,
    }

    struct Client {
        stream: TcpStream,
        received: Vec<u8>,
    }

    impl Client {
        fn connect(server: &RunningServer) -> Self {
            let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            Self {
                stream,
                received: Vec::new(),
            }
        }

        fn send(&mut self, text: &str) {
            self.stream.write_all(text.as_bytes()).unwrap();
        }

        fn answer(&mut self) -> Answer {
            loop {
                if let Some(answer) = self.parse() {
                    return answer;
                }
                let mut chunk = [0; 4096];
                let read = self.stream.read(&mut chunk).unwrap();
                assert!(read > 0, "сервер закрыл соединение, не ответив");
                self.received.extend_from_slice(&chunk[..read]);
            }
        }

        fn parse(&mut self) -> Option<Answer> {
            let mut headers = [httparse::EMPTY_HEADER; 16];
            let mut response = httparse::Response::new(&mut headers);
            let httparse::Status::Complete(head) = response.parse(&self.received).unwrap() else {
                return None;
            };
            let header = |name: &str| {
                response
                    .headers
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case(name))
                    .map(|header| String::from_utf8_lossy(header.value).into_owned())
                    .unwrap_or_default()
            };
            let length = header("content-length").parse().unwrap_or(0);
            if self.received.len() < head + length {
                return None;
            }
            let answer = Answer {
                status: response.code.unwrap(),
                connection: header("connection"),
                body: String::from_utf8_lossy(&self.received[head..head + length]).into_owned(),
            };
            self.received.drain(..head + length);
            Some(answer)
        }

        fn hung_up(&mut self) -> bool {
            match self.stream.read(&mut [0; 1]) {
                Ok(read) => read == 0,
                Err(error) => !matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ),
            }
        }
    }

    /// Opt-in official conformance fixture: actual MCP tool router, disposable Core.
    #[test]
    #[ignore = "started by scripts/mcp-conformance-smoke.sh"]
    fn isolated_conformance_fixture_serves_temporary_core() {
        use crate::app_core::{Core, CoreEvent};
        use std::fs;
        use std::path::PathBuf;
        use std::time::Instant;

        let port_file = PathBuf::from(
            std::env::var_os("AKAI_MCP_PORT_FILE").expect("AKAI_MCP_PORT_FILE must be set"),
        );
        let stop_file = PathBuf::from(
            std::env::var_os("AKAI_MCP_STOP_FILE").expect("AKAI_MCP_STOP_FILE must be set"),
        );
        let token = std::env::var("AKAI_MCP_TEST_BEARER").expect("fixture bearer required");
        assert!(token.len() >= 32);
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let vault = dir.path().join("vault");
        fs::create_dir_all(&appdata).unwrap();
        fs::create_dir_all(&vault).unwrap();
        let root = fs::canonicalize(&vault).unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        // SDK smoke uses the same File Gate and revisions as installed Akula.
        // All data and credentials are disposable.
        crate::files::gate::create(
            &core,
            &root.join("Conformance.md"),
            "# Conformance\n\nTemporary note.\n",
            crate::history::Source::Agent,
            None,
        )
        .unwrap();
        if let Some(path) = std::env::var_os("AKAI_MCP_WORKSPACE_FILE") {
            fs::write(path, root.to_string_lossy().as_bytes()).unwrap();
        }
        let tool_core = Arc::clone(&core);
        let server = RunningServer::start(
            0,
            token,
            Arc::new(move |name, args| crate::mcp::tools::call(&tool_core, name, args)),
        )
        .unwrap();
        fs::write(&port_file, server.port().to_string()).unwrap();
        eprintln!("AKAI_MCP_FIXTURE_STARTED port={} temporary=true", server.port());

        let start = Instant::now();
        while !stop_file.is_file() && start.elapsed() < Duration::from_secs(420) {
            thread::sleep(Duration::from_millis(100));
        }
        let stopped = stop_file.is_file();
        server.stop();
        core.shutdown();
        assert!(stopped, "isolated conformance fixture timed out");
    }

    #[test]
    fn answers_an_authorized_request_with_json() {
        let (server, url) = server();
        let response = ureq::post(&url)
            .set("Authorization", &bearer())
            .send_string(&initialize())
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.header("Content-Type").unwrap().contains("application/json"));
        let body: Value = serde_json::from_str(&response.into_string().unwrap()).unwrap();
        assert_eq!(body["result"]["serverInfo"]["name"], "aquilum");
        server.stop();
    }

    #[test]
    fn explicit_http_protocol_version_is_checked_before_any_tool_call() {
        let invoked = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&invoked);
        let (server, url) = start(Arc::new(move |_name, _args| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Null)
        }));
        let call = tool_call("read_note");

        let allowed = ureq::post(&url)
            .set("Authorization", &bearer())
            .set("MCP-Protocol-Version", protocol::PROTOCOL_VERSION)
            .send_string(&call)
            .unwrap();
        assert_eq!(allowed.status(), 200);
        assert_eq!(invoked.load(Ordering::SeqCst), 1);

        for requested in ["2025-11-25", "2026-07-28", "2099-01-01", ""] {
            let denied = ureq::post(&url)
                .set("Authorization", &bearer())
                .set("MCP-Protocol-Version", requested)
                .send_string(&call)
                .unwrap_err();
            assert_eq!(status(Err(denied)), Some(400), "{requested}");
            assert_eq!(invoked.load(Ordering::SeqCst), 1);
        }

        let mut duplicate = Client::connect(&server);
        duplicate.send(&sized(
            &call,
            "MCP-Protocol-Version: 2025-06-18\r\nMCP-Protocol-Version: 2026-07-28\r\n",
        ));
        assert_eq!(duplicate.answer().status, 400);
        assert_eq!(invoked.load(Ordering::SeqCst), 1);

        // Authorization is evaluated first. A remote process cannot learn
        // an accepted protocol version without a valid bearer.
        let unauthorized = ureq::post(&url)
            .set("MCP-Protocol-Version", "2026-07-28")
            .send_string(&call)
            .unwrap_err();
        assert_eq!(status(Err(unauthorized)), Some(401));
        server.stop();
    }

    #[test]
    fn rejects_a_request_without_a_valid_token() {
        let (server, url) = server();
        let anonymous = ureq::post(&url).send_string(&initialize()).unwrap_err();
        let wrong = ureq::post(&url)
            .set("Authorization", "Bearer another-token")
            .send_string(&initialize())
            .unwrap_err();
        assert!(matches!(anonymous, ureq::Error::Status(401, _)));
        assert!(matches!(wrong, ureq::Error::Status(401, _)));
        server.stop();
    }

    #[test]
    fn scoped_agent_http_rejects_legacy_token_and_exposes_no_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut credential_store =
            AgentCredentialStore::open(&dir.path().join("agent-creds.sqlite3")).unwrap();
        let mut agent_a = credential_store.issue_principal("A", None).unwrap();
        let mut agent_b = credential_store.issue_principal("B", None).unwrap();
        let bearer_a = agent_a.take_bearer_once().unwrap();
        let bearer_b = agent_b.take_bearer_once().unwrap();
        let store = Arc::new(Mutex::new(credential_store));
        let server = RunningServer::start_scoped_deny_all(0, Arc::clone(&store)).unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", server.port());

        // No credential, unknown credential or the historical global bearer
        // may ever trigger an MCP tool in a scoped listener.
        for unauthorized in [None, Some(TOKEN), Some("unknown-credential")] {
            let mut request = ureq::post(&url);
            if let Some(token) = unauthorized {
                request = request.set("Authorization", &format!("Bearer {token}"));
            }
            let denied = request.send_string(&initialize()).unwrap_err();
            assert_eq!(status(Err(denied)), Some(401));
        }

        for token in [bearer_a.as_str(), bearer_b.as_str()] {
            let bearer_header = format!("Bearer {token}");
            let initialized: Value = ureq::post(&url)
                .set("Authorization", &bearer_header)
                .send_string(&initialize())
                .unwrap()
                .into_string()
                .map(|raw| serde_json::from_str(&raw).unwrap())
                .unwrap();
            assert_eq!(initialized["result"]["serverInfo"]["name"], "aquilum");
            let listed: Value = ureq::post(&url)
                .set("Authorization", &bearer_header)
                .send_string(&json!({
                    "jsonrpc":"2.0","id":3,"method":"tools/list"
                }).to_string())
                .unwrap()
                .into_string()
                .map(|raw| serde_json::from_str(&raw).unwrap())
                .unwrap();
            assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 0);
            let denied_tool: Value = ureq::post(&url)
                .set("Authorization", &bearer_header)
                .send_string(&tool_call("read_note"))
                .unwrap()
                .into_string()
                .map(|raw| serde_json::from_str(&raw).unwrap())
                .unwrap();
            assert_eq!(denied_tool["result"]["isError"], true);
            assert!(denied_tool["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("MCP_AGENT_SCOPE_DENIED"));
        }

        // A long-lived TCP connection must not cache a principal/session
        // across HTTP requests. Revoke the same bearer *between* two writes
        // to exactly the same socket: the second request must fail with 401.
        let mut persistent = Client::connect(&server);
        let body = initialize();
        let same_socket = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n\r\n{}",
            bearer_b.as_str(), body.len(), body
        );
        persistent.send(&same_socket);
        assert_eq!(persistent.answer().status, 200);
        store.lock().unwrap().revoke(agent_b.credential).unwrap();
        persistent.send(&same_socket);
        assert_eq!(persistent.answer().status, 401);
        assert!(persistent.hung_up());

        // A fresh connection with the same revoked token also fails.
        let revoked = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", bearer_b.as_str()))
            .send_string(&initialize())
            .unwrap_err();
        assert_eq!(status(Err(revoked)), Some(401));
        assert_eq!(
            ureq::post(&url)
                .set("Authorization", &format!("Bearer {}", bearer_a.as_str()))
                .send_string(&initialize())
                .unwrap()
                .status(),
            200
        );
        server.stop();
    }

    #[test]
    fn scoped_http_does_not_promote_user_minted_vault_grants_into_read_access() {
        use super::super::agent_vault_grants::{AgentVaultGrantStore, HostVaultRegistration};
        use crate::plugins::{Capability, GrantScope, VaultId};
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let x = dir.path().join("vault-x");
        let y = dir.path().join("vault-y");
        fs::create_dir(&x).unwrap();
        fs::create_dir(&y).unwrap();
        fs::write(x.join("secret.md"), "BROKERLESS_SECRET_X").unwrap();
        fs::write(y.join("secret.md"), "BROKERLESS_SECRET_Y").unwrap();

        let mut credentials =
            AgentCredentialStore::open(&dir.path().join("agents.sqlite3")).unwrap();
        let mut agent_a = credentials.issue_principal("A", None).unwrap();
        let mut agent_b = credentials.issue_principal("B", None).unwrap();
        let bearer_a = agent_a.take_bearer_once().unwrap();
        let bearer_b = agent_b.take_bearer_once().unwrap();

        // Both receipts are SELF-ISSUED test claims. They must never be
        // interpreted as evidence of a protected installer-owned broker.
        let claim_x =
            HostVaultRegistration::from_host_custody(VaultId::new_for_host(), [31; 32])
                .unwrap();
        let claim_y =
            HostVaultRegistration::from_host_custody(VaultId::new_for_host(), [47; 32])
                .unwrap();
        let grants_path = dir.path().join("grants.sqlite3");
        let mut grants = AgentVaultGrantStore::provision_new(&grants_path).unwrap();
        grants.register_host_vault(claim_x).unwrap();
        grants.register_host_vault(claim_y).unwrap();
        grants
            .grant_notes_read(
                agent_a.principal,
                claim_x,
                Capability::NotesRead,
                GrantScope::WholeVault,
                0,
                None,
            )
            .unwrap();
        grants
            .grant_notes_read(
                agent_b.principal,
                claim_y,
                Capability::NotesRead,
                GrantScope::WholeVault,
                0,
                None,
            )
            .unwrap();
        assert!(grants
            .authorize_notes_read(
                credentials.authenticate(bearer_a.as_str()).unwrap(),
                claim_x,
            )
            .is_ok());
        assert!(grants
            .authorize_notes_read(
                credentials.authenticate(bearer_b.as_str()).unwrap(),
                claim_y,
            )
            .is_ok());
        drop(grants);
        let persisted = AgentVaultGrantStore::open_existing(&grants_path).unwrap();
        assert!(persisted
            .authorize_notes_read(
                credentials.authenticate(bearer_a.as_str()).unwrap(),
                claim_x,
            )
            .is_ok());
        drop(persisted);

        // Real HTTP admission, not an in-memory protocol mock. Valid agent
        // bearers and persisted grants alone confer NO tool authority.
        let store = Arc::new(Mutex::new(credentials));
        let server = RunningServer::start_scoped_deny_all(0, store).unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", server.port());
        for bearer in [bearer_a.as_str(), bearer_b.as_str()] {
            let header = format!("Bearer {bearer}");
            let listing: Value = serde_json::from_str(
                &ureq::post(&url)
                    .set("Authorization", &header)
                    .send_string(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
                    .unwrap()
                    .into_string()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(listing["result"]["tools"], json!([]));

            for claim in [claim_x, claim_y] {
                let request = json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {
                        "name": "read_note",
                        "arguments": {
                            "vaultId": claim.vault_id().as_uuid().to_string(),
                            "path": "secret.md"
                        }
                    }
                });
                let reply: Value = serde_json::from_str(
                    &ureq::post(&url)
                        .set("Authorization", &header)
                        .send_string(&request.to_string())
                        .unwrap()
                        .into_string()
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(reply["result"]["isError"], true);
                assert_eq!(
                    reply["result"]["content"][0]["text"],
                    "MCP_AGENT_SCOPE_DENIED"
                );
                let text = reply.to_string();
                assert!(!text.contains("BROKERLESS_SECRET_X"));
                assert!(!text.contains("BROKERLESS_SECRET_Y"));
                assert!(!text.contains("vault-x"));
                assert!(!text.contains("vault-y"));
            }

            // Mixed JSON-RPC batches must not bypass denial either.
            let batch = json!([
                {"jsonrpc": "2.0", "id": 3, "method": "tools/list"},
                {
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "tools/call",
                    "params": {"name": "read_note", "arguments": {"path": "secret.md"}}
                }
            ]);
            let reply: Value = serde_json::from_str(
                &ureq::post(&url)
                    .set("Authorization", &header)
                    .send_string(&batch.to_string())
                    .unwrap()
                    .into_string()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(reply[0]["result"]["tools"], json!([]));
            assert_eq!(reply[1]["result"]["isError"], true);
            assert_eq!(
                reply[1]["result"]["content"][0]["text"],
                "MCP_AGENT_SCOPE_DENIED"
            );
        }
        server.stop();
    }

    /// AKAI-HOST-FASTPATH-V1: real localhost HTTP/MCP request/response proof.
    /// Every root receipt is SELF-ISSUED IN A TEST. No installed issuer exists.
    #[test]
    fn scoped_host_v1_real_http_two_agents_read_and_revoke() {
        use crate::plugins::{Capability, GrantScope, VaultId};
        use crate::sync::filegate_lock_probe::PinnedProbeRoot;
        use std::fs;
        use std::path::Path;

        fn root(path: &Path, receipt: HostVaultRegistration) -> HostPinnedReadRoot {
            let probe = PinnedProbeRoot::open(path).unwrap();
            let identity = probe.identity();
            drop(probe);
            HostPinnedReadRoot::from_test_host_claim(path, receipt, identity).unwrap()
        }
        fn rpc(url: &str, key: &str, body: Value) -> Value {
            let response = ureq::post(url)
                .set("Authorization", &format!("Bearer {key}"))
                .send_string(&body.to_string())
                .unwrap()
                .into_string()
                .unwrap();
            serde_json::from_str(&response).unwrap()
        }
        fn read(url: &str, key: &str, vault: VaultId, path: &str) -> Value {
            rpc(
                url,
                key,
                json!({
                    "jsonrpc": "2.0", "id": 11, "method": "tools/call",
                    "params": { "name": "read_note", "arguments": {
                        "vaultId": vault.as_uuid().to_string(), "note": path
                    }}
                }),
            )
        }

        let dir = tempfile::tempdir().unwrap();
        let x = dir.path().join("X");
        let y = dir.path().join("Y");
        fs::create_dir(&x).unwrap();
        fs::create_dir(&y).unwrap();
        fs::write(x.join("note.md"), "SCOPED_AGENT_A_ONLY").unwrap();
        fs::write(y.join("note.md"), "SCOPED_AGENT_B_ONLY").unwrap();

        let receipt_x =
            HostVaultRegistration::from_host_custody(VaultId::new_for_host(), [11; 32])
                .unwrap();
        let receipt_y =
            HostVaultRegistration::from_host_custody(VaultId::new_for_host(), [22; 32])
                .unwrap();
        let mut credentials =
            AgentCredentialStore::open(&dir.path().join("agents.sqlite3")).unwrap();
        let mut agent_a = credentials.issue_principal("agent-A", None).unwrap();
        let mut agent_b = credentials.issue_principal("agent-B", None).unwrap();
        let mut agent_c = credentials.issue_principal("agent-C-no-grant", None).unwrap();
        let key_a = agent_a.take_bearer_once().unwrap();
        let key_b = agent_b.take_bearer_once().unwrap();
        let key_c = agent_c.take_bearer_once().unwrap();
        let mut grants =
            AgentVaultGrantStore::provision_new(&dir.path().join("grants.sqlite3")).unwrap();
        grants.register_host_vault(receipt_x).unwrap();
        grants.register_host_vault(receipt_y).unwrap();
        let epoch_a = grants
            .grant_notes_read(
                agent_a.principal,
                receipt_x,
                Capability::NotesRead,
                GrantScope::WholeVault,
                0,
                None,
            )
            .unwrap();
        grants
            .grant_notes_read(
                agent_b.principal,
                receipt_y,
                Capability::NotesRead,
                GrantScope::WholeVault,
                0,
                None,
            )
            .unwrap();

        let store = Arc::new(Mutex::new(credentials));
        let grant_store = Arc::new(Mutex::new(grants));
        let roots = BTreeMap::from([
            (receipt_x.vault_id(), (receipt_x, root(&x, receipt_x))),
            (receipt_y.vault_id(), (receipt_y, root(&y, receipt_y))),
        ]);
        let server = RunningServer::start_scoped_read_fixture(
            0,
            ScopedReadFixture {
                credentials: Arc::clone(&store),
                grants: Arc::clone(&grant_store),
                roots,
            },
        )
        .unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", server.port());
        let list = json!({"jsonrpc":"2.0","id":9,"method":"tools/list"});
        let listed = rpc(&url, key_a.as_str(), list.clone());
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
        assert_eq!(listed["result"]["tools"][0]["name"], "read_note");
        assert_eq!(
            rpc(&url, key_c.as_str(), list.clone())["result"]["tools"],
            json!([])
        );

        for (key, vault, expected) in [
            (key_a.as_str(), receipt_x.vault_id(), "SCOPED_AGENT_A_ONLY"),
            (key_b.as_str(), receipt_y.vault_id(), "SCOPED_AGENT_B_ONLY"),
        ] {
            let outcome = read(&url, key, vault, "note.md");
            assert_eq!(outcome["result"]["isError"], false);
            let inner: Value = serde_json::from_str(
                outcome["result"]["content"][0]["text"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(inner["content"], expected);
            assert_eq!(inner["revision"].as_str().unwrap().len(), 64);
        }
        for (key, vault, path) in [
            (key_a.as_str(), receipt_y.vault_id(), "note.md"),
            (key_b.as_str(), receipt_x.vault_id(), "note.md"),
            (key_c.as_str(), receipt_x.vault_id(), "note.md"),
            (key_a.as_str(), receipt_x.vault_id(), "../Y/note.md"),
        ] {
            let forbidden = read(&url, key, vault, path);
            assert_eq!(forbidden["result"]["isError"], true);
            assert_eq!(
                forbidden["result"]["content"][0]["text"],
                "MCP_AGENT_SCOPE_DENIED"
            );
            assert!(!forbidden.to_string().contains("SCOPED_AGENT_"));
        }
        let forbidden_write = rpc(
            &url,
            key_a.as_str(),
            json!({
                "jsonrpc":"2.0", "id":10, "method":"tools/call",
                "params":{
                    "name":"update_note",
                    "arguments":{"note":"note.md","content":"attack"}
                }
            }),
        );
        assert_eq!(forbidden_write["result"]["isError"], true);
        assert_eq!(
            fs::read_to_string(x.join("note.md")).unwrap(),
            "SCOPED_AGENT_A_ONLY"
        );

        // An opened root must not authorize a replacement at the old path.
        // Windows may permit or refuse rename despite a pinned HANDLE.
        let renamed = dir.path().join("X-renamed");
        if fs::rename(&x, &renamed).is_ok() {
            fs::create_dir(&x).unwrap();
            fs::write(x.join("note.md"), "SUBSTITUTED_UNTRUSTED_BYTES").unwrap();
            let substituted = read(&url, key_a.as_str(), receipt_x.vault_id(), "note.md");
            assert_eq!(substituted["result"]["isError"], true);
            assert_eq!(
                substituted["result"]["content"][0]["text"],
                "MCP_AGENT_SCOPE_DENIED"
            );
            assert!(!substituted.to_string().contains("SUBSTITUTED_UNTRUSTED_BYTES"));
        }

        // Previously successful HTTP tokens must not survive grant revoke.
        let epoch = grant_store
            .lock()
            .unwrap()
            .revoke_notes_read(agent_a.principal, receipt_x, epoch_a);
        assert!(epoch.is_ok());
        assert_eq!(
            rpc(&url, key_a.as_str(), list)["result"]["tools"],
            json!([])
        );
        assert_eq!(
            read(&url, key_a.as_str(), receipt_x.vault_id(), "note.md")
                ["result"]["isError"],
            true
        );
        assert_eq!(
            read(&url, key_b.as_str(), receipt_y.vault_id(), "note.md")
                ["result"]["isError"],
            false
        );
        // Credential revoke must additionally return HTTP 401 on new calls.
        store.lock().unwrap().revoke(agent_b.credential).unwrap();
        let denied = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", key_b.as_str()))
            .send_string(&initialize())
            .unwrap_err();
        assert_eq!(status(Err(denied)), Some(401));

        server.stop();
    }

    #[test]
    fn host_allowlist_rejects_dns_rebinding_and_port_spoofing() {
        let port = 8787;
        for allowed in [
            "127.0.0.1",
            "127.0.0.1:8787",
            "localhost",
            "LOCALHOST:8787",
            "[::1]:8787",
        ] {
            assert!(
                is_loopback_host(allowed, port),
                "valid local host: {allowed}"
            );
        }
        for forbidden in [
            "",
            "attacker.example",
            "attacker.example:8787",
            "localhost.evil.example:8787",
            "127.0.0.1.evil.example",
            "localhost@evil.example",
            "127.0.0.1, attacker.example",
            "http://127.0.0.1:8787",
            "localhost:8788",
            "localhost:0",
            "localhost:",
            "localhost:8787:1",
            "[::1]@evil.example",
            "[::ffff:127.0.0.1]:8787",
            "2130706433:8787",
            "0x7f000001:8787",
        ] {
            assert!(
                !is_loopback_host(forbidden, port),
                "unexpectedly allowed Host: {forbidden}"
            );
        }
    }

    #[test]
    fn rejects_foreign_missing_and_duplicate_host_before_tool_dispatch() {
        let invoked = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&invoked);
        let (server, _) = start(Arc::new(move |_name, _arguments| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Null)
        }));
        let body = tool_call("read_note");

        let send_with_host = |host_headers: &str| -> Answer {
            let mut client = Client::connect(&server);
            let message = format!(
                "POST /mcp HTTP/1.1\r\n{host_headers}Authorization: Bearer {TOKEN}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            client.send(&message);
            client.answer()
        };

        for valid in [
            format!("Host: localhost:{}\r\n", server.port()),
            format!("Host: 127.0.0.1:{}\r\n", server.port()),
            "Host: localhost\r\n".to_owned(),
        ] {
            assert_eq!(send_with_host(&valid).status, 200);
        }
        assert_eq!(invoked.load(Ordering::SeqCst), 3);

        let wrong_port = if server.port() == u16::MAX {
            server.port() - 1
        } else {
            server.port() + 1
        };
        for forbidden in [
            "Host: attacker.example\r\n".to_owned(),
            format!("Host: attacker.example:{}\r\n", server.port()),
            format!("Host: localhost.evil.example:{}\r\n", server.port()),
            format!("Host: localhost:{wrong_port}\r\n"),
            "Host: 127.0.0.1@attacker.example\r\n".to_owned(),
        ] {
            assert_eq!(send_with_host(&forbidden).status, 403, "{forbidden}");
            assert_eq!(invoked.load(Ordering::SeqCst), 3);
        }
        for malformed in [
            String::new(),
            "Host: \r\n".to_owned(),
            "Host: localhost\r\nHost: localhost\r\n".to_owned(),
        ] {
            assert_eq!(send_with_host(&malformed).status, 400, "{malformed}");
            assert_eq!(invoked.load(Ordering::SeqCst), 3);
        }
        server.stop();
    }

    #[test]
    fn accepts_the_bearer_scheme_in_any_case() {
        let (server, url) = server();
        let response = ureq::post(&url)
            .set("Authorization", &format!("BEARER {TOKEN}"))
            .send_string(&initialize())
            .unwrap();
        assert_eq!(response.status(), 200);
        server.stop();
    }

    #[test]
    fn rejects_a_foreign_origin_even_with_a_valid_token() {
        let (server, url) = server();
        let error = ureq::post(&url)
            .set("Authorization", &bearer())
            .set("Origin", "https://evil.example")
            .send_string(&initialize())
            .unwrap_err();
        assert!(matches!(error, ureq::Error::Status(403, _)));
        server.stop();
    }

    #[test]
    fn accepts_a_notification_without_a_body() {
        let (server, url) = server();
        let response = ureq::post(&url)
            .set("Authorization", &bearer())
            .send_string(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string())
            .unwrap();
        assert_eq!(response.status(), 202);
        server.stop();
    }

    #[test]
    fn refuses_methods_other_than_post() {
        let (server, url) = server();
        let error = ureq::get(&url)
            .set("Authorization", &bearer())
            .call()
            .unwrap_err();
        let ureq::Error::Status(405, response) = error else {
            panic!("ожидался 405, пришло {error}");
        };
        assert_eq!(response.header("Allow"), Some("POST"));
        server.stop();
    }

    #[test]
    fn accepts_only_real_loopback_origins() {
        assert!(is_loopback_origin("http://127.0.0.1:1420"));
        assert!(is_loopback_origin("http://localhost"));
        assert!(is_loopback_origin("http://[::1]:8080"));
        assert!(!is_loopback_origin("http://localhost.evil.example"));
        assert!(!is_loopback_origin("http://127.0.0.1.evil.example"));
        assert!(!is_loopback_origin("http://[::1]@evil.example"));
        assert!(!is_loopback_origin("https://localhost"));
        assert!(!is_loopback_origin("null"));
    }

    #[test]
    fn rejects_an_origin_that_only_looks_like_loopback() {
        let (server, url) = server();
        let error = ureq::post(&url)
            .set("Authorization", &bearer())
            .set("Origin", "http://localhost.evil.example")
            .send_string(&initialize())
            .unwrap_err();
        assert!(matches!(error, ureq::Error::Status(403, _)));
        server.stop();
    }

    #[test]
    fn answers_every_short_lived_connection_while_a_tool_is_slow() {
        let (server, url) = start(slow_tool(Duration::from_millis(20)));
        for attempt in 0..40 {
            let response = ureq::post(&url)
                .set("Authorization", &bearer())
                .set("Connection", "close")
                .send_string(&tool_call("slow"))
                .unwrap_or_else(|error| panic!("запрос {attempt}: {error}"));
            assert_eq!(response.status(), 200);
            assert_eq!(response.header("Connection"), Some("close"));
        }
        server.stop();
    }

    #[test]
    fn reclaims_idle_keep_alive_before_refusing_new_work() {
        use std::time::Instant;
        let (server, _) = server();
        let mut held = (0..MAX_CONNECTIONS)
            .map(|_| {
                let mut client = Client::connect(&server);
                client.send(&sized(&tool_call("held"), ""));
                assert_eq!(client.answer().status, 200);
                client
            })
            .collect::<Vec<_>>();

        // Wait until each completed response is registered as idle. Selecting
        // its ID proves the exact client reclaimed, not merely that a fresh
        // response happened after a timeout.
        let mut reclaimed_id = None;
        for _ in 0..2500 {
            let slots = lock(&server.endpoint.open);
            if slots.count == MAX_CONNECTIONS && slots.idle.len() == MAX_CONNECTIONS {
                reclaimed_id = slots.idle.first().map(|idle| idle.id as usize);
                break;
            }
            drop(slots);
            thread::sleep(Duration::from_millis(2));
        }
        let reclaimed_id = reclaimed_id.expect("16 keep-alive connections became idle");
        assert!(reclaimed_id < MAX_CONNECTIONS);

        let start = Instant::now();
        let mut waiting = Client::connect(&server);
        waiting.send(&sized(&tool_call("waiting"), ""));
        let answer = waiting.answer();
        assert_eq!(answer.status, 200);
        assert!(answer.body.contains("waiting"));
        assert!(start.elapsed() < Duration::from_secs(3), "idle reclamation was too slow");
        assert!(held[reclaimed_id].hung_up(), "idle victim wasn't disconnected");
        assert_eq!(lock(&server.endpoint.open).count, MAX_CONNECTIONS);
        drop(waiting);
        drop(held);
        server.stop();
    }

    #[test]
    fn partial_header_after_keep_alive_stops_idle_reclamation() {
        let (server, _) = server();
        let mut held = (0..MAX_CONNECTIONS)
            .map(|_| {
                let mut client = Client::connect(&server);
                client.send(&sized(&initialize(), ""));
                assert_eq!(client.answer().status, 200);
                client
            })
            .collect::<Vec<_>>();

        let mut all_idle = false;
        for _ in 0..2500 {
            if lock(&server.endpoint.open).idle.len() == MAX_CONNECTIONS {
                all_idle = true;
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(all_idle, "pooled connections never became idle");

        let partial = sized(&tool_call("partially-framed"), "");
        assert!(partial.starts_with("POST"));
        held[0].send(&partial[..1]);

        let mut became_active = false;
        for _ in 0..2500 {
            let slots = lock(&server.endpoint.open);
            if slots.idle.len() == MAX_CONNECTIONS - 1
                && slots.idle.iter().all(|idle| idle.id != 0)
            {
                became_active = true;
                break;
            }
            drop(slots);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(became_active, "first header byte must clear idle registration");

        let mut incoming = Client::connect(&server);
        incoming.send(&sized(&tool_call("fresh"), ""));
        assert_eq!(incoming.answer().status, 200);

        // The in-progress header is not a reclamation victim: finish and
        // process it while a *different* already-idle socket was evicted.
        held[0].send(&partial[1..]);
        let original = held[0].answer();
        assert_eq!(original.status, 200);
        assert!(original.body.contains("partially-framed"));

        drop(incoming);
        drop(held);
        server.stop();
    }

    #[test]
    fn returns_bounded_503_when_all_sockets_are_mid_request() {
        use std::time::Instant;
        let (server, _) = server();
        let mut active = (0..MAX_CONNECTIONS)
            .map(|_| {
                let mut client = Client::connect(&server);
                // Authorized, but intentionally incomplete body: this socket
                // is NOT idle and must not be evicted under pressure.
                client.send(&head("Content-Length: 1024\r\n"));
                client
            })
            .collect::<Vec<_>>();
        let mut occupied = false;
        for _ in 0..2500 {
            let slots = lock(&server.endpoint.open);
            if slots.count == MAX_CONNECTIONS && slots.idle.is_empty() {
                occupied = true;
                break;
            }
            drop(slots);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(occupied, "all 16 request slots were occupied");
        let mut overflow = Client::connect(&server);
        let start = Instant::now();
        overflow.send(&sized(&initialize(), ""));
        let refused = overflow.answer();
        assert_eq!((refused.status, refused.connection.as_str()), (503, "close"));
        assert!(start.elapsed() < Duration::from_secs(3), "overload waited for a busy socket");
        assert!(refused.body.contains("соединения заняты"));
        assert!(overflow.hung_up());

        // A busy socket must survive the overload. Completing its body
        // returns the JSON-RPC parse error, not a forced TCP disconnect.
        active[0].send(&" ".repeat(1024));
        let response = active[0].answer();
        assert_eq!(response.status, 200);
        assert!(response.body.contains("-32700"));
        drop(overflow);
        drop(active);
        server.stop();
    }

    #[test]
    fn keeps_the_connection_between_requests() {
        let (server, _) = server();
        let mut client = Client::connect(&server);
        for name in ["first", "second"] {
            client.send(&sized(&tool_call(name), ""));
            let answer = client.answer();
            assert_eq!((answer.status, answer.connection.as_str()), (200, "keep-alive"));
            assert!(answer.body.contains(name));
        }
        server.stop();
    }

    #[test]
    fn tells_a_pooled_client_how_long_an_idle_connection_lives() {
        let (server, url) = server();
        let agent = ureq::agent();
        for _ in 0..20 {
            let response = agent
                .post(&url)
                .set("Authorization", &bearer())
                .send_string(&initialize())
                .unwrap();
            assert_eq!(response.header("Keep-Alive"), Some("timeout=60"));
            response.into_string().unwrap();
        }
        server.stop();
    }

    #[test]
    fn closes_the_connection_when_the_client_asks() {
        let (server, _) = server();
        let mut client = Client::connect(&server);
        client.send(&sized(&initialize(), "Connection: close\r\n"));
        let answer = client.answer();
        assert_eq!((answer.status, answer.connection.as_str()), (200, "close"));
        let _ = client.stream.shutdown(Shutdown::Write);
        assert!(client.hung_up());
        server.stop();
    }

    #[test]
    fn waits_for_the_body_after_100_continue() {
        let (server, _) = server();
        let mut client = Client::connect(&server);
        let body = initialize();
        client.send(&head(&format!("Content-Length: {}\r\nExpect: 100-continue\r\n", body.len())));
        assert_eq!(client.answer().status, 100);
        let (first, rest) = body.split_at(10);
        client.send(first);
        thread::sleep(Duration::from_millis(20));
        client.send(rest);
        assert!(client.answer().body.contains("serverInfo"));
        server.stop();
    }

    #[test]
    fn reads_a_chunked_body_with_trailers() {
        let (server, _) = server();
        let mut client = Client::connect(&server);
        let body = initialize();
        let (first, rest) = body.split_at(10);
        client.send(&format!(
            "{}{:x}\r\n{first}\r\n{:x}\r\n{rest}\r\n0\r\nX-Trailer: yes\r\n\r\n",
            head("Transfer-Encoding: chunked\r\n"),
            first.len(),
            rest.len()
        ));
        assert!(client.answer().body.contains("serverInfo"));
        server.stop();
    }

    #[test]
    fn refuses_a_chunked_body_over_the_limit() {
        let (server, url) = server();
        let endless = io::repeat(b' ').take(MAX_BODY_BYTES + 1);
        let error = ureq::post(&url)
            .set("Authorization", &bearer())
            .send(endless)
            .unwrap_err();
        let ureq::Error::Status(413, response) = error else {
            panic!("ожидался 413, пришло {error}");
        };
        assert_eq!(response.header("Connection"), Some("close"));
        server.stop();
    }

    #[test]
    fn answers_an_oversized_body_instead_of_resetting() {
        let (server, url) = server();
        let error = ureq::post(&url)
            .set("Authorization", &bearer())
            .send_string(&"x".repeat(MAX_BODY_BYTES as usize + 1))
            .unwrap_err();
        assert!(matches!(error, ureq::Error::Status(413, _)), "{error}");
        server.stop();
    }

    mod throughput_probe {
        include!("server/throughput_probe.rs");
    }

    mod real_file_gate {
        include!("server/real_file_gate.rs");
    }

    #[test]
    fn parallel_reads_are_bounded_and_never_overlap_exclusive_writes() {
        use std::sync::Barrier;

        let active_reads = Arc::new(AtomicUsize::new(0));
        let active_writer = Arc::new(AtomicBool::new(false));
        let max_reads = Arc::new(AtomicUsize::new(0));
        let violation = Arc::new(AtomicBool::new(false));
        let (reads, writer, peak, conflict) = (
            Arc::clone(&active_reads),
            Arc::clone(&active_writer),
            Arc::clone(&max_reads),
            Arc::clone(&violation),
        );
        let (server, url) = start(Arc::new(move |name, _| {
            if name == "read_note" {
                let running = reads.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(running, Ordering::SeqCst);
                if writer.load(Ordering::SeqCst) {
                    conflict.store(true, Ordering::SeqCst);
                }
                thread::sleep(Duration::from_millis(45));
                if writer.load(Ordering::SeqCst) {
                    conflict.store(true, Ordering::SeqCst);
                }
                reads.fetch_sub(1, Ordering::SeqCst);
            } else {
                if writer.swap(true, Ordering::SeqCst) || reads.load(Ordering::SeqCst) != 0 {
                    conflict.store(true, Ordering::SeqCst);
                }
                thread::sleep(Duration::from_millis(20));
                if reads.load(Ordering::SeqCst) != 0 {
                    conflict.store(true, Ordering::SeqCst);
                }
                writer.store(false, Ordering::SeqCst);
            }
            Ok(Value::Null)
        }));

        let calls = [
            "read_note", "update_note", "read_note", "read_note", "read_note",
            "read_note", "update_note", "read_note", "read_note", "read_note",
        ];
        let gate = Arc::new(Barrier::new(calls.len() + 1));
        let workers = calls
            .into_iter()
            .map(|name| {
                let url = url.clone();
                let gate = Arc::clone(&gate);
                thread::spawn(move || {
                    gate.wait();
                    ureq::post(&url)
                        .set("Authorization", &bearer())
                        .send_string(&tool_call(name))
                        .is_ok_and(|response| response.status() == 200)
                })
            })
            .collect::<Vec<_>>();
        gate.wait();
        assert!(workers.into_iter().all(|worker| worker.join().unwrap()));
        assert!(!violation.load(Ordering::SeqCst), "write overlapped a read or a write");
        assert!(max_reads.load(Ordering::SeqCst) >= 2, "independent reads did not overlap");
        assert!(
            max_reads.load(Ordering::SeqCst) <= MAX_PARALLEL_READS,
            "reader concurrency exceeded configured limit"
        );
        server.stop();
    }

    #[test]
    fn stop_drains_admitted_reads_and_rejects_queued_mutation() {
        let finished_reads = Arc::new(AtomicUsize::new(0));
        let started_writes = Arc::new(AtomicUsize::new(0));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (started, entered) = mpsc::channel();
        let (finished, writes, release_in_tool) = (
            Arc::clone(&finished_reads),
            Arc::clone(&started_writes),
            Arc::clone(&release),
        );
        let (server, url) = start(Arc::new(move |name, _| {
            if name == "read_note" {
                let _ = started.send(());
                let (state, changed) = &*release_in_tool;
                let mut released = lock(state);
                while !*released {
                    released = changed.wait(released).unwrap_or_else(PoisonError::into_inner);
                }
                finished.fetch_add(1, Ordering::SeqCst);
            } else {
                writes.fetch_add(1, Ordering::SeqCst);
            }
            Ok(Value::Null)
        }));

        let readers = (0..MAX_PARALLEL_READS)
            .map(|_| {
                let url = url.clone();
                thread::spawn(move || {
                    status(
                        ureq::post(&url)
                            .set("Authorization", &bearer())
                            .send_string(&tool_call("read_note")),
                    )
                })
            })
            .collect::<Vec<_>>();
        for _ in 0..MAX_PARALLEL_READS {
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
        }

        let writer = thread::spawn(move || {
            status(
                ureq::post(&url)
                    .set("Authorization", &bearer())
                    .send_string(&tool_call("update_note")),
            )
        });
        let mut waiting = false;
        for _ in 0..1000 {
            if lock(&server.endpoint.turn.state).waiting_writers == 1 {
                waiting = true;
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(waiting, "writer was not queued behind readers");

        let stopping = Arc::clone(&server.endpoint);
        let stopping_thread = thread::spawn(move || server.stop());
        for _ in 0..1000 {
            if stopping.stopping.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(stopping.stopping.load(Ordering::SeqCst));
        let (state, changed) = &*release;
        *lock(state) = true;
        changed.notify_all();

        stopping_thread.join().unwrap();
        for read in readers {
            assert_eq!(read.join().unwrap(), Some(200));
        }
        assert_eq!(writer.join().unwrap(), Some(503));
        assert_eq!(finished_reads.load(Ordering::SeqCst), MAX_PARALLEL_READS);
        assert_eq!(started_writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn waiting_writers_get_fifo_admission() {
        let gate = Arc::new(DispatchGate::new());
        let stopping = Arc::new(AtomicBool::new(false));
        let held_read = gate.acquire(true, &stopping).unwrap();
        let (sent, received) = mpsc::channel();
        let mut writers = Vec::new();

        for number in 0..4 {
            let waiting_gate = Arc::clone(&gate);
            let waiting_stop = Arc::clone(&stopping);
            let sender = sent.clone();
            writers.push(thread::spawn(move || {
                let _permit = waiting_gate.acquire(false, &waiting_stop).unwrap();
                sender.send(number).unwrap();
                thread::sleep(Duration::from_millis(10));
            }));
            let mut queued = false;
            for _ in 0..1000 {
                if lock(&gate.state).waiting_writers == number + 1 {
                    queued = true;
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            assert!(queued, "writer {number} was not queued in time");
        }

        drop(held_read);
        for expected in 0..4 {
            assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), expected);
        }
        for writer in writers {
            writer.join().unwrap();
        }
    }

    #[test]
    fn queued_writer_takes_priority_over_later_readers() {
        let gate = Arc::new(DispatchGate::new());
        let stopping = Arc::new(AtomicBool::new(false));
        let active = gate.acquire(true, &stopping).unwrap();
        let (sender, receiver) = mpsc::channel();
        let write_gate = Arc::clone(&gate);
        let write_stopping = Arc::clone(&stopping);
        let writer_sender = sender.clone();
        let writer = thread::spawn(move || {
            let _permit = write_gate.acquire(false, &write_stopping).unwrap();
            writer_sender.send("writer").unwrap();
            thread::sleep(Duration::from_millis(10));
        });

        let mut seen_waiting = false;
        for _ in 0..1000 {
            if lock(&gate.state).waiting_writers == 1 {
                seen_waiting = true;
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(seen_waiting, "writer never queued");

        let read_gate = Arc::clone(&gate);
        let read_stopping = Arc::clone(&stopping);
        let reader = thread::spawn(move || {
            let _permit = read_gate.acquire(true, &read_stopping).unwrap();
            sender.send("reader").unwrap();
        });
        drop(active);
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), "writer");
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), "reader");
        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn runs_one_tool_call_at_a_time() {
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let (tool_running, tool_most) = (Arc::clone(&running), Arc::clone(&most));
        let (server, url) = start(Arc::new(move |_name: &str, _arguments: &Value| {
            let now = tool_running.fetch_add(1, Ordering::SeqCst) + 1;
            tool_most.fetch_max(now, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(15));
            tool_running.fetch_sub(1, Ordering::SeqCst);
            Ok(Value::Null)
        }));
        let clients = (0..4)
            .map(|_| {
                let url = url.clone();
                thread::spawn(move || {
                    (0..3).all(|_| {
                        ureq::post(&url)
                            .set("Authorization", &bearer())
                            .send_string(&tool_call("tool"))
                            .is_ok_and(|response| response.status() == 200)
                    })
                })
            })
            .collect::<Vec<_>>();
        assert!(clients.into_iter().all(|client| client.join().unwrap()));
        assert_eq!(most.load(Ordering::SeqCst), 1);
        server.stop();
    }

    #[test]
    fn stop_lets_the_running_call_answer_and_refuses_the_rest() {
        let calls = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let (started, running) = mpsc::channel();
        let (tool_calls, tool_finished) = (Arc::clone(&calls), Arc::clone(&finished));
        let (server, url) = start(Arc::new(move |_name: &str, _arguments: &Value| {
            tool_calls.fetch_add(1, Ordering::SeqCst);
            let _ = started.send(());
            thread::sleep(Duration::from_millis(150));
            tool_finished.store(true, Ordering::SeqCst);
            Ok(Value::Null)
        }));
        let mut idle = Client::connect(&server);
        idle.send(&sized(&initialize(), ""));
        assert_eq!(idle.answer().status, 200);

        let first_url = url.clone();
        let first = thread::spawn(move || {
            status(
                ureq::post(&first_url)
                    .set("Authorization", &bearer())
                    .send_string(&tool_call("slow")),
            )
        });
        running.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued = thread::spawn(move || {
            status(
                ureq::post(&url)
                    .set("Authorization", &bearer())
                    .send_string(&tool_call("queued")),
            )
        });
        thread::sleep(Duration::from_millis(30));
        server.stop();

        assert!(finished.load(Ordering::SeqCst), "остановка дождалась идущего вызова");
        assert_eq!(first.join().unwrap(), Some(200), "ответ идущего вызова дошёл");
        assert_eq!(queued.join().unwrap(), Some(503), "вызов из очереди отклонён");
        idle.send(&sized(&initialize(), ""));
        let refused = idle.answer();
        assert_eq!((refused.status, refused.connection.as_str()), (503, "close"));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "после остановки инструменты не запускаются");
    }

    #[test]
    fn frees_the_port_after_stop() {
        let (server, url) = server();
        let port = server.port();
        ureq::post(&url)
            .set("Authorization", &bearer())
            .send_string(&initialize())
            .unwrap();
        server.stop();
        assert!(ureq::post(&url).send_string(&initialize()).is_err());
        let caller: SharedToolCaller = Arc::new(|_name: &str, _arguments: &Value| Ok(Value::Null));
        RunningServer::start(port, TOKEN.to_owned(), caller)
            .expect("порт освобождён и сервер поднимается снова")
            .stop();
    }
}
