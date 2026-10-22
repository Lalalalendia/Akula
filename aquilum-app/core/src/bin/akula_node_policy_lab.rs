//! AK-DATA-CTRL-01D: opt-in Linux loopback process, not a public server.
//! Actual on-disk document reads reuse ReadCatalog, and signed anti-memes may
//! ONLY change the separate appdata-only policy cache. No File Gate writes.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_SCOPE_LAB_REFUSED: Linux only");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("AKULA_SCOPE_LAB_REFUSED: {error}");
        std::process::exit(2);
    }
}

#[cfg(target_os = "linux")]
#[path = "../node_scope_revocation_lab.rs"]
mod node_scope_revocation_lab;

#[cfg(target_os = "linux")]
mod linux {
    use aquilum_core::node_catalog_lab::ReadCatalog;
    use super::node_scope_revocation_lab::{
        decode_hex, encode_hex, Ingest, ScopedRevocations,
    };
    use aquilum_core::sync::VaultId;
    use serde_json::{json, Value};
    use std::env;
    use std::fs;
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::str::FromStr;
    use std::time::Duration;

    const MAX_LINE: usize = 240;
    const MAX_REQUESTS: usize = 80;

    struct Config {
        root: PathBuf,
        appdata: PathBuf,
        vault: VaultId,
        note: String,
        admin: [u8; 32],
        token: [u8; 32],
        port: u16,
        requests: usize,
    }

    fn config() -> Result<Config, String> {
        let mut opts = std::collections::BTreeMap::new();
        let mut args = env::args().skip(1);
        while let Some(option) = args.next() {
            if !matches!(option.as_str(),
                "--vault-root" | "--appdata" | "--vault-id" |
                "--note" | "--admin-key" | "--port" | "--max-requests")
            {
                return Err("unknown argument".into());
            }
            let value = args.next().ok_or("missing argument value")?;
            if opts.insert(option.clone(), value).is_some() {
                return Err("duplicate argument".into());
            }
        }
        let get = |key: &str| opts.get(key).ok_or("missing required argument");
        let root = PathBuf::from(get("--vault-root")?);
        let appdata = PathBuf::from(get("--appdata")?);
        let vault = VaultId::from_str(get("--vault-id")?)
            .map_err(|_| "invalid scope ID")?;
        let note = get("--note")?.clone();
        if note.len() > 128 || note.is_empty()
            || !note.ends_with(".md")
            || Path::new(&note).components().count() != 1
            || note.contains('\\') || note.bytes().any(|b| b.is_ascii_control())
        {
            return Err("note must be one top-level markdown filename".into());
        }
        let admin = decode_hex::<32>(get("--admin-key")?)
            .ok_or("invalid pinned administrator public key")?;
        let token = decode_hex::<32>(&env::var("AKULA_POLICY_LAB_TOKEN")
            .map_err(|_| "missing AKULA_POLICY_LAB_TOKEN")?)
            .ok_or("expected exactly 32-byte hexadecimal bearer")?;
        let port = opts.get("--port").map(String::as_str).unwrap_or("0")
            .parse::<u16>().map_err(|_| "invalid port")?;
        let requests = opts.get("--max-requests").map(String::as_str).unwrap_or("20")
            .parse::<usize>().map_err(|_| "invalid request budget")?;
        if !(1..=MAX_REQUESTS).contains(&requests) {
            return Err("requests must be between 1 and 80".into());
        }
        Ok(Config { root, appdata, vault, note, admin, token, port, requests })
    }

    fn bounded_line(reader: &mut BufReader<TcpStream>) -> io::Result<String> {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            if reader.read_exact(&mut byte).is_err() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "short line"));
            }
            if byte[0] == b'\n' { break; }
            if line.len() == MAX_LINE || byte[0] == b'\r' || byte[0] == 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "bad line"));
            }
            line.push(byte[0]);
        }
        String::from_utf8(line)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad UTF-8"))
    }

    fn authorized(reader: &mut BufReader<TcpStream>, expected: &[u8; 32]) -> bool {
        let Ok(line) = bounded_line(reader) else { return false; };
        let Some(candidate) = line.strip_prefix("AUTH ") else { return false; };
        let Some(token) = decode_hex::<32>(candidate) else { return false; };
        ring::constant_time::verify_slices_are_equal(expected, &token).is_ok()
    }

    fn handle(
        command: &str,
        catalog: &ReadCatalog,
        policy: &mut ScopedRevocations,
    ) -> Value {
        if policy.is_poisoned() {
            return json!({"status":"unavailable"});
        }
        let Some((operation, arg)) = command.split_once(' ') else {
            return json!({"status":"invalid"});
        };
        let known = |doc: &str| catalog.list().iter().any(|entry| entry.id == doc);
        match operation {
            "READ" if decode_hex::<32>(arg).is_some() => {
                if !known(arg) { return json!({"status":"not_found"}); }
                if policy.is_revoked(arg) {
                    return json!({
                        "status":"blocked",
                        "proof": policy.signed_proof(arg),
                    });
                }
                match catalog.read(arg) {
                    Ok(file) => json!({
                        "status":"allowed",
                        "id":file.id,
                        "content":file.content,
                        "rawBlake3":file.raw_revision,
                    }),
                    Err(_) => json!({"status":"unavailable"}),
                }
            }
            "PROOF" if decode_hex::<32>(arg).is_some() => {
                if !known(arg) { return json!({"status":"not_found"}); }
                match policy.signed_proof(arg) {
                    Some(proof) => json!({"status":"proof","proof":proof}),
                    None => json!({"status":"absent"}),
                }
            }
            "BAN" if arg.len() == 208 => {
                // Prevent a bearer-only peer from filling the small local
                // witness DB with IDs not in the already operator-pinned catalog.
                let Some(raw) = decode_hex::<104>(arg) else {
                    return json!({"status":"rejected"});
                };
                let document = encode_hex(&raw[..32]);
                if !known(&document) { return json!({"status":"rejected"}); }
                match policy.ingest(arg) {
                    Ok(Ingest::Applied) => json!({"status":"applied"}),
                    Ok(Ingest::Duplicate) => json!({"status":"duplicate"}),
                    Ok(Ingest::Stale { correction }) =>
                        json!({"status":"stale","proof":correction}),
                    Err(_) => json!({"status":"rejected"}),
                }
            }
            _ => json!({"status":"invalid"}),
        }
    }

    fn serve(
        mut stream: TcpStream,
        token: &[u8; 32],
        catalog: &ReadCatalog,
        policy: &mut ScopedRevocations,
    ) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let answer = {
            let mut reader = BufReader::new(stream.try_clone()?);
            if !authorized(&mut reader, token) {
                json!({"status":"unauthorized"})
            } else {
                match bounded_line(&mut reader) {
                    Ok(command) => handle(&command, catalog, policy),
                    Err(_) => json!({"status":"invalid"}),
                }
            }
        };
        let mut response = serde_json::to_vec(&answer)
            .map_err(|_| io::Error::other("response encoding failed"))?;
        response.push(b'\n');
        stream.write_all(&response)
    }

    pub fn run() -> Result<(), String> {
        let cfg = config()?;
        let root = fs::canonicalize(&cfg.root).map_err(|_| "vault root absent")?;
        let appdata = fs::canonicalize(&cfg.appdata).map_err(|_| "state root absent")?;
        if !root.is_dir() || !appdata.is_dir()
            || root.starts_with(&appdata) || appdata.starts_with(&root)
        {
            return Err("vault and appdata must be existing, separate directories".into());
        }

        let catalog = ReadCatalog::open(&cfg.root, cfg.vault, &[&cfg.note])
            .map_err(|_| "cannot pin operator-allowlisted document")?;
        let mut policy = ScopedRevocations::open(
            &appdata.join("node-scope-revocations.lab.sqlite"),
            cfg.vault,
            cfg.admin,
        )?;
        // Lab network exposure is intentionally fixed to localhost even when
        // the user asks for a different bind address.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, cfg.port))
            .map_err(|_| "cannot bind loopback policy peer")?;
        println!("AKULA_SCOPE_LAB_BOUND={}",
            listener.local_addr().map_err(|_| "missing bound listener")?);
        io::stdout().flush().map_err(|_| "cannot announce lab listener")?;
        for _ in 0..cfg.requests {
            let (stream, _) = listener.accept().map_err(|_| "listener failed")?;
            let _ = serve(stream, &cfg.token, &catalog, &mut policy);
        }
        Ok(())
    }
}
