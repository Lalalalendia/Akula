//! Standalone, manually addressed LAN proof-of-concept. No files or Sync Core.
use akula_roha_transport::{AllowedPeers, StayaTransport, lan_discovery, pairing};
use iroh::{EndpointAddr, EndpointId, SecretKey};
use std::{
    collections::BTreeSet,
    env,
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, SocketAddr},
    path::Path,
    str::FromStr,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

fn decode_key(s: &str) -> Result<SecretKey, String> {
    if s.len() != 64 {
        return Err("secret key must be 64 hex characters".into());
    }
    let mut raw = [0u8; 32];
    for (i, byte) in raw.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| "invalid secret hex")?;
    }
    Ok(SecretKey::from_bytes(&raw))
}

fn key_hex(key: &SecretKey) -> String {
    key.to_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// Only file-backed private keys are accepted. Never put private key hex
/// in shell arguments; command-line arguments are visible to other processes.
fn secret_from_arg(value: &str) -> Result<SecretKey, String> {
    let Some(path) = value.strip_prefix('@') else {
        return Err("private key must be supplied as @KEY_PATH".into());
    };
    if path.is_empty() {
        return Err("missing secret key file path".into());
    }
    let file = File::open(path).map_err(|e| format!("cannot open key file: {e}"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("cannot inspect key file: {e}"))?;
    if !metadata.is_file() || metadata.len() > 128 {
        return Err("invalid key file type or size".into());
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err("key file must not be readable by group or others (mode 0600)".into());
    }
    let mut encoded = String::new();
    file.take(129)
        .read_to_string(&mut encoded)
        .map_err(|e| format!("cannot read key file: {e}"))?;
    decode_key(encoded.trim_end_matches(&['\r', '\n'][..]))
}

fn save_key_file(path: &Path, key: &SecretKey) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|e| format!("cannot create key file: {e}"))?;
    file.write_all(key_hex(key).as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("cannot save secret key: {e}"))
}

/// Bounded read of an untrusted public contact. It never grants access.
fn read_contact_file(path: &str) -> Result<EndpointId, String> {
    let file = File::open(path).map_err(|e| format!("cannot open contact file: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot inspect contact file: {e}"))?;
    if !meta.is_file() || meta.len() > pairing::MAX_RECORD_BYTES as u64 {
        return Err("invalid contact file type or size".into());
    }
    let mut text = String::new();
    file.take(pairing::MAX_RECORD_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("cannot read contact file: {e}"))?;
    pairing::parse_contact(&text)
}

/// Explicitly confirmed public key pin. Local OS users with write access to
/// this file can change it, so this is a lab allowlist, not signed membership.
fn read_trusted_peer_file(path: &str) -> Result<EndpointId, String> {
    let symlink_meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot inspect trusted peer path: {e}"))?;
    if !symlink_meta.file_type().is_file() {
        return Err("trusted peer record must be a regular file, not a symlink".into());
    }
    #[cfg(unix)]
    if symlink_meta.permissions().mode() & 0o077 != 0 {
        return Err("trusted peer record requires owner-only permissions".into());
    }
    let file = File::open(path).map_err(|e| format!("cannot open trusted peer record: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot inspect trusted peer record: {e}"))?;
    if !meta.is_file() || meta.len() > pairing::MAX_RECORD_BYTES as u64 {
        return Err("invalid trusted peer record type or size".into());
    }
    let mut text = String::new();
    file.take(pairing::MAX_RECORD_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("cannot read trusted peer record: {e}"))?;
    pairing::parse_trusted_peer(&text)
}

/// Only a confirmed contact can be saved as trusted. Create a new local file
/// atomically with respect to path existence; never silently replace a pin.
fn write_trusted_peer_file(path: &Path, id: EndpointId) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|e| format!("cannot create trusted peer record: {e}"))?;
    file.write_all(pairing::export_trusted_peer(id).as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("cannot save trusted peer record: {e}"))
}

fn peer_from_arg(kind: &str, value: &str) -> Result<EndpointId, String> {
    if kind.ends_with("-trusted") {
        read_trusted_peer_file(value)
    } else {
        parse_id(value)
    }
}

fn parse_id(s: &str) -> Result<EndpointId, String> {
    EndpointId::from_str(s).map_err(|e| format!("invalid peer id: {e}"))
}

fn usage() -> &'static str {
    "usage:\n  staya-lan show-contact @KEY_PATH > contact.txt\n  staya-lan inspect-contact CONTACT_PATH\n  staya-lan confirm-contact CONTACT_PATH VERIFIED_FULL_ENDPOINT_ID TRUST_OUTPUT_PATH\n  staya-lan gen-key-file KEY_PATH\n  staya-lan serve-trusted @KEY_PATH TRUSTED_PEER_FILE BIND_IP:UDP_PORT\n  staya-lan ping-trusted @KEY_PATH TRUSTED_PEER_FILE SERVER_IP:UDP_PORT\n  staya-lan serve-mdns-trusted @KEY_PATH TRUSTED_PEER_FILE BIND_IP:UDP_PORT ANNOUNCE_IPV4\n  staya-lan ping-discovered-trusted @KEY_PATH TRUSTED_PEER_FILE SECONDS_1_TO_20\n  staya-lan serve @KEY_PATH ALLOWED_PEER_ID BIND_IP:UDP_PORT\n  staya-lan ping @KEY_PATH SERVER_PEER_ID SERVER_IP:UDP_PORT\n  staya-lan serve-mdns @KEY_PATH ALLOWED_PEER_ID BIND_IP:UDP_PORT ANNOUNCE_IPV4\n  staya-lan ping-discovered @KEY_PATH ALLOWED_PEER_ID SECONDS_1_TO_20\nDiscovery only runs on explicit request; mDNS broadcasts your public endpoint ID on the LAN.\nDo not publish or commit the private key files.\n"
}

/// The received mDNS address is never trusted as authentication. QUIC
/// verifies the explicitly enrolled endpoint key before accepting PONG.
async fn send_probe(local: &StayaTransport, addr: EndpointAddr) -> Result<(), String> {
    let conn = local.connect(addr).await?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    send.write_all(b"akula-ping-v0")
        .await
        .map_err(|e| e.to_string())?;
    send.finish().map_err(|e| e.to_string())?;
    let received = tokio::time::timeout(Duration::from_secs(15), recv.read_to_end(64))
        .await
        .map_err(|_| "reply timeout")?
        .map_err(|e| e.to_string())?;
    if received.as_slice() != b"akula-pong-v0" {
        conn.close(1u8.into(), b"wrong response");
        return Err("wrong response".into());
    }
    conn.close(0u8.into(), b"ping complete");
    Ok(())
}

async fn serve(
    key: SecretKey,
    allowed: EndpointId,
    socket: SocketAddr,
    announce_ip: Option<IpAddr>,
) -> Result<(), String> {
    let transport = StayaTransport::bind_on(key, AllowedPeers::new([allowed]), socket).await?;
    let _mdns = if let Some(ip) = announce_ip {
        let advertised = SocketAddr::new(ip, socket.port());
        Some(lan_discovery::advertise(transport.id(), advertised)?)
    } else {
        None
    };
    println!("endpoint_id={}", transport.id());
    println!("local_udp_sockets={:?}", transport.bound_sockets());
    println!(
        "relay=disabled discovery={} allowed_peer={allowed}",
        _mdns.is_some()
    );
    loop {
        let conn = transport.accept_one().await?;
        let (mut send, mut recv) = tokio::time::timeout(Duration::from_secs(15), conn.accept_bi())
            .await
            .map_err(|_| "stream accept timeout")?
            .map_err(|e| e.to_string())?;
        let msg = tokio::time::timeout(Duration::from_secs(15), recv.read_to_end(64))
            .await
            .map_err(|_| "receive timeout")?
            .map_err(|e| e.to_string())?;
        if msg.as_slice() != b"akula-ping-v0" {
            conn.close(1u8.into(), b"unexpected payload");
            continue;
        }
        send.write_all(b"akula-pong-v0")
            .await
            .map_err(|e| e.to_string())?;
        send.finish().map_err(|e| e.to_string())?;
        let _ = tokio::time::timeout(Duration::from_secs(15), conn.closed()).await;
        println!("ping_ok remote={}", conn.remote_id());
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("show-contact") if args.len() == 3 => {
            let key = secret_from_arg(&args[2])?;
            print!("{}", pairing::export_contact(key.public()));
        }
        Some("inspect-contact") if args.len() == 3 => {
            let id = read_contact_file(&args[2])?;
            println!("untrusted_contact_endpoint_id={id}");
            println!("not_enrolled=true; verify independently before trusting");
        }
        Some("confirm-contact") if args.len() == 5 => {
            let id = read_contact_file(&args[2])?;
            let expected = parse_id(&args[3])?;
            if id != expected {
                return Err("contact ID mismatch: no trust record created".into());
            }
            write_trusted_peer_file(Path::new(&args[4]), id)?;
            println!("peer_pin_saved=true file_access=false");
        }
        Some("gen-key-file") if args.len() == 3 => {
            let key = SecretKey::generate();
            save_key_file(Path::new(&args[2]), &key)?;
            println!("endpoint_id={}", key.public());
            eprintln!("Private key saved. Protect its location; never upload it.");
        }
        Some("serve" | "serve-trusted") if args.len() == 5 => {
            let key = secret_from_arg(&args[2])?;
            let allowed = peer_from_arg(&args[1], &args[3])?;
            let socket: SocketAddr = args[4].parse().map_err(|_| "invalid bind socket")?;
            serve(key, allowed, socket, None).await?;
        }
        Some("serve-mdns" | "serve-mdns-trusted") if args.len() == 6 => {
            let key = secret_from_arg(&args[2])?;
            let allowed = peer_from_arg(&args[1], &args[3])?;
            let socket: SocketAddr = args[4].parse().map_err(|_| "invalid bind socket")?;
            let ip: IpAddr = args[5].parse().map_err(|_| "invalid LAN advertise IP")?;
            serve(key, allowed, socket, Some(ip)).await?;
        }
        Some("ping" | "ping-trusted") if args.len() == 5 => {
            let key = secret_from_arg(&args[2])?;
            let server = peer_from_arg(&args[1], &args[3])?;
            let socket: SocketAddr = args[4].parse().map_err(|_| "invalid peer socket")?;
            let local = StayaTransport::bind(key, AllowedPeers::new([server])).await?;
            let addr = EndpointAddr::new(server).with_ip_addr(socket);
            let outcome = send_probe(&local, addr).await;
            if outcome.is_ok() {
                println!("direct_lan_ping_ok peer={server} relay=disabled");
            }
            local.close().await;
            outcome?;
        }
        Some("ping-discovered" | "ping-discovered-trusted") if args.len() == 5 => {
            let key = secret_from_arg(&args[2])?;
            let server = peer_from_arg(&args[1], &args[3])?;
            let seconds: u64 = args[4].parse().map_err(|_| "invalid discovery duration")?;
            if !(1..=20).contains(&seconds) {
                return Err("discovery window must be 1..20 seconds".into());
            }
            // This blocking discovery is intentionally done BEFORE opening the
            // current-thread Tokio endpoint, so QUIC timers are never stalled.
            let peers = BTreeSet::from([server]);
            let candidates = lan_discovery::scan(&peers, Duration::from_secs(seconds))?;
            if candidates.is_empty() {
                return Err("no permitted peers were discovered; use manual ping".into());
            }
            let local = StayaTransport::bind(key, AllowedPeers::new([server])).await?;
            let mut connected = false;
            for candidate in candidates {
                let addr = EndpointAddr::new(server).with_ip_addr(candidate.address);
                if send_probe(&local, addr).await.is_ok() {
                    println!("direct_lan_ping_ok peer={server} via=mdns relay=disabled");
                    connected = true;
                    break;
                }
            }
            local.close().await;
            if !connected {
                return Err("all discovered addresses failed QUIC verification".into());
            }
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_hex_roundtrip_and_bad_keys() {
        let k = SecretKey::generate();
        let hex: String = k.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(decode_key(&hex).unwrap().public(), k.public());
        assert!(decode_key("bad").is_err());
        assert!(decode_key(&"gg".repeat(32)).is_err());
    }

    #[test]
    fn key_file_roundtrip_rejects_overwrite_and_raw_argument() {
        let key = SecretKey::generate();
        let path = env::temp_dir().join(format!(
            "staya-lan-key-{}-{}.key",
            std::process::id(),
            key.public()
        ));
        save_key_file(&path, &key).expect("create private key file");
        let reference = format!("@{}", path.display());
        assert_eq!(
            secret_from_arg(&reference)
                .expect("read secret file")
                .public(),
            key.public()
        );
        assert!(save_key_file(&path, &SecretKey::generate()).is_err());
        assert!(secret_from_arg(&key_hex(&key)).is_err());
        #[cfg(unix)]
        {
            use std::fs::Permissions;
            std::fs::set_permissions(&path, Permissions::from_mode(0o644))
                .expect("make key file insecure");
            assert!(secret_from_arg(&reference).is_err());
        }
        std::fs::remove_file(path).expect("remove test key");
    }

    #[test]
    fn key_file_rejects_invalid_bytes_and_missing_path() {
        let key = SecretKey::generate();
        let path = env::temp_dir().join(format!(
            "staya-lan-bad-key-{}-{}.key",
            std::process::id(),
            key.public()
        ));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let mut file = opts.open(&path).expect("create key file");
        file.write_all(&[b'x'; 129]).expect("write oversized input");
        drop(file);
        let reference = format!("@{}", path.display());
        assert!(secret_from_arg(&reference).is_err());
        assert!(secret_from_arg("@").is_err());
        std::fs::remove_file(path).expect("remove invalid key");
    }
}
