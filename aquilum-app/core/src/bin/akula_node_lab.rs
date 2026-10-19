//! Opt-in Linux-only D1 personal Node LAB. Not a released public service.
//! No remote writes, no arbitrary URL paths, no MCP tools, no external bind.
//! Serve ONE operator-allowlisted top-level Markdown file using pinned dirfd.
//! Core has private writable appdata, separate from the served read-only vault.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_NODE_D1_REFUSED: Linux only");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("AKULA_NODE_D1_REFUSED: {error}");
        std::process::exit(2);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use aquilum_core::{Core, CoreEvent, EventSink};
    use rustix::fs::{open, openat, Mode, OFlags};
    use serde_json::json;
    use std::env;
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    const MAX_HEADERS: usize = 8192;
    const MAX_FILE_BYTES: u64 = 64 * 1024;
    const MAX_REQUESTS: usize = 32;

    struct Config {
        root: PathBuf,
        appdata: PathBuf,
        note: String,
        port: u16,
        requests: usize,
        token: String,
    }

    fn parse_config() -> Result<Config, String> {
        let mut root = None;
        let mut appdata = None;
        let mut note = None;
        let mut port = None;
        let mut requests = None;
        let mut args = env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
            let slot = match flag.as_str() {
                "--vault-root" => &mut root,
                "--appdata" => &mut appdata,
                "--note" => &mut note,
                "--port" => &mut port,
                "--max-requests" => &mut requests,
                _ => return Err("unknown command-line option".into()),
            };
            if slot.replace(value).is_some() {
                return Err("duplicate command-line option".into());
            }
        }
        let root = PathBuf::from(root.ok_or("missing --vault-root")?);
        let appdata = PathBuf::from(appdata.ok_or("missing --appdata")?);
        let note = note.ok_or("missing --note")?;
        let components = Path::new(&note).components().collect::<Vec<_>>();
        if components.len() != 1
            || !matches!(components[0], Component::Normal(_))
            || !note.ends_with(".md")
            || note.len() > 128
        {
            return Err("--note must be a top-level .md filename (not a path)".into());
        }
        let port: u16 = port.unwrap_or_else(|| "0".to_owned()).parse()
            .map_err(|_| "invalid --port")?;
        let requests: usize = requests.unwrap_or_else(|| "8".to_owned()).parse()
            .map_err(|_| "invalid --max-requests")?;
        if !(1..=MAX_REQUESTS).contains(&requests) {
            return Err("--max-requests must be 1..=32".into());
        }
        let token = env::var("AKULA_NODE_D1_TOKEN")
            .map_err(|_| "AKULA_NODE_D1_TOKEN is required")?;
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("AKULA_NODE_D1_TOKEN must be 64 random hexadecimal characters".into());
        }
        Ok(Config { root, appdata, note, port, requests, token })
    }

    fn response(status: u16, value: serde_json::Value) -> Vec<u8> {
        let reason = match status {
            200 => "OK", 400 => "Bad Request", 401 => "Unauthorized",
            403 => "Forbidden", 404 => "Not Found", 405 => "Method Not Allowed",
            408 => "Request Timeout", 409 => "Conflict", 413 => "Content Too Large",
            422 => "Unprocessable Content", 431 => "Request Header Fields Too Large",
            _ => "Internal Server Error",
        };
        let body = serde_json::to_vec(&value).expect("JSON object");
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
            body.len()
        );
        let mut out = head.into_bytes();
        out.extend_from_slice(&body);
        out
    }

    struct Request {
        method: String,
        path: String,
        bearer: Vec<u8>,
    }

    fn request(stream: &mut TcpStream) -> Result<Request, u16> {
        let mut bytes = [0u8; MAX_HEADERS];
        let mut len = 0;
        loop {
            if len == MAX_HEADERS { return Err(431); }
            let n = match stream.read(&mut bytes[len..]) {
                Ok(0) => return Err(400),
                Ok(n) => n,
                Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => return Err(408),
                Err(_) => return Err(400),
            };
            len += n;
            if bytes[..len].windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut parsed = httparse::Request::new(&mut headers);
        let consumed = match parsed.parse(&bytes[..len]) {
            Ok(httparse::Status::Complete(n)) => n,
            _ => return Err(400),
        };
        // Reject pipelined requests and request bodies in this tiny lab.
        if consumed != len || parsed.version != Some(1) { return Err(400); }
        if parsed.headers.iter().any(|h| h.name.eq_ignore_ascii_case("Content-Length") ||
                h.name.eq_ignore_ascii_case("Transfer-Encoding")) { return Err(400); }
        let auth = parsed.headers.iter().filter(|h| h.name.eq_ignore_ascii_case("Authorization"))
            .collect::<Vec<_>>();
        if auth.len() != 1 { return Err(401); }
        let Some(bearer) = auth[0].value.strip_prefix(b"Bearer ") else { return Err(401); };
        Ok(Request {
            method: parsed.method.unwrap_or_default().to_owned(),
            path: parsed.path.unwrap_or_default().to_owned(),
            bearer: bearer.to_vec(),
        })
    }

    fn safe_read(root: &File, note: &str) -> Result<(String, String), u16> {
        // The root is opened once, by the operator before listening.
        // Filename is operator allowlisted at startup; never obtained from HTTP.
        let owned = openat(
            root, note, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()
        ).map_err(|_| 403u16)?;
        let mut file = File::from(owned);
        let before = file.metadata().map_err(|_| 403u16)?;
        if !before.is_file() { return Err(403); }
        if before.len() > MAX_FILE_BYTES { return Err(413); }
        let mut bytes = Vec::new();
        (&mut file).take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|_| 403u16)?;
        if bytes.len() as u64 > MAX_FILE_BYTES { return Err(413); }
        let after = file.metadata().map_err(|_| 409u16)?;
        if before.dev() != after.dev() || before.ino() != after.ino() ||
            before.len() != after.len() || before.mtime_nsec() != after.mtime_nsec() ||
            before.mtime() != after.mtime() || before.ctime() != after.ctime() ||
            before.ctime_nsec() != after.ctime_nsec() || after.len() != bytes.len() as u64 {
            return Err(409);
        }
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let content = String::from_utf8(bytes).map_err(|_| 422u16)?;
        Ok((content, hash))
    }

    fn serve(stream: &mut TcpStream, root: &File, config: &Config) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let result = match request(stream) {
            Err(status) => response(status, json!({"error": "request refused"})),
            Ok(req) => {
                // Authentication precedes all endpoint routing, file checks and metadata.
                let allowed = ring::constant_time::verify_slices_are_equal(
                    &req.bearer, config.token.as_bytes()
                ).is_ok();
                if !allowed {
                    response(401, json!({"error": "unauthorized"}))
                } else if req.method != "GET" {
                    response(405, json!({"error": "GET only"}))
                } else if req.path == "/health" {
                    response(200, json!({"status": "ok", "profile": "node-d1-lab"}))
                } else if req.path == "/v1/note" {
                    match safe_read(root, &config.note) {
                        Ok((content, hash)) => response(200, json!({
                            "note": config.note, "content": content, "rawBlake3": hash
                        })),
                        Err(status) => response(status, json!({"error": "read refused"})),
                    }
                } else {
                    response(404, json!({"error": "not found"}))
                }
            }
        };
        stream.write_all(&result)
    }

    pub fn run() -> Result<(), String> {
        let cfg = parse_config()?;
        // Deny direct path overlap BEFORE creating any Core state files.
        let root_real = fs::canonicalize(&cfg.root).map_err(|e| format!("vault: {e}"))?;
        let state_real = fs::canonicalize(&cfg.appdata).map_err(|e| format!("appdata: {e}"))?;
        if !root_real.is_dir() || !state_real.is_dir() ||
            root_real.starts_with(&state_real) || state_real.starts_with(&root_real) {
            return Err("vault and writable appdata must be disjoint existing directories".into());
        }
        let root = File::from(
            open(&cfg.root, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW |
                OFlags::CLOEXEC, Mode::empty()).map_err(|e| format!("pin vault dirfd: {e}"))?
        );
        let pinned = root.metadata().map_err(|e| format!("vault fd: {e}"))?;
        let expected = fs::metadata(&root_real).map_err(|e| format!("vault stat: {e}"))?;
        if pinned.dev() != expected.dev() || pinned.ino() != expected.ino() {
            return Err("vault root changed while pinning".into());
        }
        let events: Arc<dyn EventSink> = Arc::new(|_event: CoreEvent| {});
        let core = Core::open(&cfg.appdata, events);
        if core.mcp.status().running {
            core.shutdown();
            return Err("refusing pre-running mutable MCP".into());
        }
        // Deliberately hardcoded LOOPBACK. No option to bind LAN/WAN.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, cfg.port))
            .map_err(|e| format!("loopback bind: {e}"))?;
        println!("AKULA_NODE_D1_BOUND={}", listener.local_addr().map_err(|e| e.to_string())?);
        io::stdout().flush().map_err(|e| e.to_string())?;
        for _ in 0..cfg.requests {
            let (mut connection, _) = listener.accept().map_err(|e| e.to_string())?;
            // One bounded request per connection. No shared mutable RPC dispatcher.
            let _ = serve(&mut connection, &root, &cfg);
        }
        core.shutdown();
        Ok(())
    }
}
