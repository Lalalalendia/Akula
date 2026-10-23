//! AK-P2P-BETA-01: one explicitly selected file on a directly reachable LAN.
//! Opt-in Windows companion, never a background sync or a Vault/File Gate ACK.
use akula_staya_integration::live_identity::{
    require_pinned_live_peer, with_protected_iroh_secret,
};
use aquilum_core::{
    p2p_device_identity::peer_pin,
    p2p_inbox_lab::{MAX_INBOX_ITEM_BYTES, save_verified_stage_as_new},
};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, endpoint::presets};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use uuid::Uuid;

#[path = "p2p_one_file_beta/inbox.rs"]
mod inbox;

const ALPN: &[u8] = b"akula/p2p-one-file-lan-beta/1";
const REQUEST: &[u8] = b"AKP2_GET_ONE_V1";
const ACK: &[u8] = b"AKP2_VERIFIED_NEW_FILE_V1";
const MAGIC: &[u8; 8] = b"AKP2ONE1";
const HEADER_BYTES: usize = 48;
const MAX_FRAME_BYTES: usize = MAX_INBOX_ITEM_BYTES as usize + HEADER_BYTES;
const USAGE: &str = "usage: akula-p2p-one-file-beta send PEER_ID ABSOLUTE_SOURCE | receive PEER_ID LAN_IP:UDP_PORT ABSOLUTE_NEW_DESTINATION";

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Send {
        peer: String,
        source: PathBuf,
    },
    Receive {
        peer: String,
        address: SocketAddr,
        destination: PathBuf,
    },
    // Native-only explicit opt-in: use third-party n0 discovery/relay.
    // No LAN or WAN mode silently changes routing under the same button.
    SendInternet {
        peer: String,
        source: PathBuf,
    },
    // Not CLI-addressable: Native stores verified data only in its Inbox.
    // None means explicitly selected Internet/N0; Some means direct LAN.
    ReceiveInbox {
        peer: String,
        address: Option<SocketAddr>,
    },
}

fn parse_peer(value: &str) -> Result<EndpointId, String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("peer key must be exactly 64 lowercase hex characters".into());
    }
    let id = EndpointId::from_str(value).map_err(|_| "invalid Iroh peer ID")?;
    if id.as_bytes() == &[0u8; 32] {
        return Err("zero public key is forbidden".into());
    }
    Ok(id)
}

fn parse_mode(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let args: Vec<_> = args.into_iter().collect();
    match args.as_slice() {
        [mode, peer, source] if mode == "send" => {
            parse_peer(peer)?;
            let source = PathBuf::from(source);
            if !source.is_absolute() {
                return Err("select an absolute source path".into());
            }
            Ok(Mode::Send {
                peer: peer.clone(),
                source,
            })
        }
        [mode, peer, address, destination] if mode == "receive" => {
            parse_peer(peer)?;
            let address: SocketAddr = address
                .parse()
                .map_err(|_| "use a numeric LAN IP:UDP_PORT, not a hostname")?;
            if address.port() == 0 {
                return Err("UDP port cannot be zero".into());
            }
            let destination = PathBuf::from(destination);
            if !destination.is_absolute() || destination.file_name().is_none() {
                return Err("choose an absolute NEW destination filename".into());
            }
            Ok(Mode::Receive {
                peer: peer.clone(),
                address,
                destination,
            })
        }
        _ => Err(USAGE.into()),
    }
}

fn selected_peer(peer: &str) -> Result<EndpointId, String> {
    let expected = parse_peer(peer)?;
    peer_pin::require_pinned_remote(peer, expected.as_bytes())
        .map_err(|error| format!("Windows protected peer is not confirmed: {error}"))?;
    Ok(expected)
}

fn live_peer(connection: &iroh::endpoint::Connection, peer: &str) -> Result<(), String> {
    require_pinned_live_peer(connection, peer)
        .map_err(|error| format!("TLS-authenticated peer rejected or revoked: {error}"))
}

/// Only an explicit local source is opened, after an authenticated request.
/// A bounded snapshot cannot grow during a transfer, and no remote paths exist.
fn source_frame(path: &Path) -> Result<Vec<u8>, String> {
    if !path.is_absolute() {
        return Err("source must be absolute".into());
    }
    let path_info =
        fs::symlink_metadata(path).map_err(|error| format!("source metadata: {error}"))?;
    if !path_info.is_file() || path_info.file_type().is_symlink() {
        return Err("source must be one normal file, never a link or directory".into());
    }
    let mut source = File::open(path).map_err(|error| format!("open source: {error}"))?;
    let size = source
        .metadata()
        .map_err(|error| format!("opened source metadata: {error}"))?
        .len();
    if !(1..=MAX_INBOX_ITEM_BYTES).contains(&size) {
        return Err("one-file beta permits 1 through 8 MiB only".into());
    }
    let mut bytes = Vec::with_capacity(size as usize);
    (&mut source)
        .take(MAX_INBOX_ITEM_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read chosen source: {error}"))?;
    if bytes.len() as u64 != size
        || source
            .metadata()
            .map_err(|error| format!("recheck source metadata: {error}"))?
            .len()
            != size
    {
        return Err("source changed during reading; nothing sent".into());
    }
    Ok(frame_for(&bytes))
}

fn frame_for(bytes: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_BYTES + bytes.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    frame.extend_from_slice(blake3::hash(bytes).as_bytes());
    frame.extend_from_slice(bytes);
    frame
}

fn decode_frame(frame: &[u8]) -> Result<(u64, [u8; 32], &[u8]), String> {
    if frame.len() < HEADER_BYTES || &frame[..8] != MAGIC {
        return Err("wrong or incomplete one-file message".into());
    }
    let size = u64::from_be_bytes(frame[8..16].try_into().expect("checked header"));
    if !(1..=MAX_INBOX_ITEM_BYTES).contains(&size) || frame.len() != HEADER_BYTES + size as usize {
        return Err("truncated, oversized or extra file data".into());
    }
    let digest: [u8; 32] = frame[16..48].try_into().expect("checked header");
    let data = &frame[HEADER_BYTES..];
    if digest == [0u8; 32] || blake3::hash(data).as_bytes() != &digest {
        return Err("file BLAKE3 mismatch; destination not published".into());
    }
    Ok((size, digest, data))
}

struct StageCleanup(PathBuf);

impl Drop for StageCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Receiver has selected this exact NEW path before network negotiation.
/// The remote peer can choose neither the destination nor a Vault operation.
/// Physical bytes are cold-verified again by existing Core create-no-replace.
#[cfg(test)]
fn save_received(frame: &[u8], destination: &Path) -> Result<(), String> {
    save_received_with(frame, destination, |_, _, _| Ok(()))
}

fn save_received_with(
    frame: &[u8],
    destination: &Path,
    before_publish: impl FnOnce(&mut File, u64, [u8; 32]) -> Result<(), String>,
) -> Result<(), String> {
    if !destination.is_absolute() || destination.file_name().is_none() {
        return Err("destination must be an absolute NEW filename".into());
    }
    let (size, digest, data) = decode_frame(frame)?;
    let parent = destination
        .parent()
        .ok_or("destination must have a parent directory")?;
    let metadata =
        fs::symlink_metadata(parent).map_err(|error| format!("destination directory: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("destination parent cannot be a symlink".into());
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err("destination already exists; no overwrite permitted".into());
    }
    let scratch_path = parent.join(format!(".akula-p2p-beta-{}.part", Uuid::new_v4()));
    let cleanup = StageCleanup(scratch_path.clone());
    let mut scratch = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&scratch_path)
        .map_err(|error| format!("create private stage: {error}"))?;
    scratch
        .write_all(data)
        .map_err(|error| format!("stage verified bytes: {error}"))?;
    scratch
        .sync_all()
        .map_err(|error| format!("sync receiver stage: {error}"))?;
    drop(scratch);
    let mut reopened = File::open(&scratch_path)
        .map_err(|error| format!("open receiver stage for cold verification: {error}"))?;
    // Record the verified local object before optional user-selected export.
    // If that export fails, the independent Inbox remains recoverable.
    before_publish(&mut reopened, size, digest)?;
    save_verified_stage_as_new(&mut reopened, size, digest, destination)
        .map_err(|error| format!("Core create-new publication rejected: {error:?}"))?;
    drop(reopened);
    drop(cleanup);
    Ok(())
}

/// Private network roundtrip; production passes a cold OS pin check closure,
/// tests may use disposable TLS identities without modifying user credentials.
async fn send_one(
    connection: &iroh::endpoint::Connection,
    source: &Path,
    check_peer: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let mut request = connection
        .accept_uni()
        .await
        .map_err(|error| format!("incoming file request: {error}"))?;
    let code = request
        .read_to_end(64)
        .await
        .map_err(|error| format!("read bounded request: {error}"))?;
    if code != REQUEST {
        return Err("unexpected request: file not opened".into());
    }
    // Recheck persistent revoke before touching any source bytes.
    check_peer()?;
    let frame = source_frame(source)?;
    let mut tx = connection
        .open_uni()
        .await
        .map_err(|error| format!("open QUIC file stream: {error}"))?;
    tx.write_all(&frame)
        .await
        .map_err(|error| format!("write bounded file: {error}"))?;
    tx.finish()
        .map_err(|error| format!("finish file stream: {error}"))?;
    let mut receipt = connection
        .accept_uni()
        .await
        .map_err(|error| format!("receiver result channel: {error}"))?;
    let reply = receipt
        .read_to_end(64)
        .await
        .map_err(|error| format!("read receiver result: {error}"))?;
    if reply != ACK {
        return Err("receiver did not confirm verified NEW file".into());
    }
    println!("RECEIVER_VERIFIED_NEW_FILE=true DURABLE_ACK=false");
    Ok(())
}

async fn receive_one(
    connection: &iroh::endpoint::Connection,
    destination: &Path,
    inbox_root: &Path,
    local_endpoint_key: [u8; 32],
    check_peer: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    receive_one_target(
        connection,
        Some(destination),
        inbox_root,
        local_endpoint_key,
        check_peer,
    )
    .await
}

// The same authenticated network request and ACK path is shared by
// operator-CLI receive (explicit destination) and Native receive (Inbox only).
async fn receive_one_target(
    connection: &iroh::endpoint::Connection,
    destination: Option<&Path>,
    inbox_root: &Path,
    local_endpoint_key: [u8; 32],
    check_peer: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    // Receiver always chooses its own destination; no remote path is trusted.
    check_peer()?;
    let mut tx = connection
        .open_uni()
        .await
        .map_err(|error| format!("open explicit request: {error}"))?;
    tx.write_all(REQUEST)
        .await
        .map_err(|error| format!("request exactly one file: {error}"))?;
    tx.finish()
        .map_err(|error| format!("finish request: {error}"))?;
    let mut stream = connection
        .accept_uni()
        .await
        .map_err(|error| format!("incoming file stream: {error}"))?;
    let bytes = stream
        .read_to_end(MAX_FRAME_BYTES)
        .await
        .map_err(|error| format!("read bounded file: {error}"))?;
    // OS revocation is rechecked before physically publishing a new file.
    check_peer()?;
    if let Some(destination) = destination {
        save_received_with(&bytes, destination, |stage, size, digest| {
            inbox::persist_received(
                stage,
                inbox_root,
                size,
                digest,
                *connection.remote_id().as_bytes(),
                local_endpoint_key,
            )
            .map(|_| ())
        })?;
    } else {
        let (size, digest, data) = decode_frame(&bytes)?;
        inbox::persist_received_payload(
            data,
            inbox_root,
            size,
            digest,
            *connection.remote_id().as_bytes(),
            local_endpoint_key,
        )?;
    }
    let mut ack = connection
        .open_uni()
        .await
        .map_err(|error| format!("open verified receipt: {error}"))?;
    ack.write_all(ACK)
        .await
        .map_err(|error| format!("write verified receipt: {error}"))?;
    ack.finish()
        .map_err(|error| format!("finish verified receipt: {error}"))?;
    println!("NEW_FILE_VERIFIED=true FILE_GATE_APPLIED=false DURABLE_ACK=false");
    Ok(())
}

/// Native receive has a numeric, user-supplied direct LAN address for an
/// independently OS-pinned peer. No relay, hostname or internet endpoint.
pub fn validate_direct_lan_address(address: SocketAddr) -> Result<(), String> {
    if address.port() == 0 {
        return Err("receiver needs a nonzero UDP port".into());
    }
    let is_local = match address.ip() {
        std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        std::net::IpAddr::V6(ip) => {
            ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback()
        }
    };
    if !is_local {
        return Err("direct LAN beta requires a local/private numeric IP".into());
    }
    Ok(())
}

/// No user-chosen destination is required: the receiver stores exactly one
/// checked file in its private permanent Inbox and never imports into Vault.
pub async fn native_receive_one(peer: String, address: SocketAddr) -> Result<(), String> {
    parse_peer(&peer)?;
    validate_direct_lan_address(address)?;
    run_mode(
        Mode::ReceiveInbox {
            peer,
            address: Some(address),
        },
        |_| {},
    )
    .await
}

/// Internet/N0 is a separate explicit UI action. No clipboard addresses,
/// no automatic discovery at application launch, no change to LAN-only mode.
pub async fn native_receive_internet_one(peer: String) -> Result<(), String> {
    parse_peer(&peer)?;
    run_mode(
        Mode::ReceiveInbox {
            peer,
            address: None,
        },
        |_| {},
    )
    .await
}

/// Explicit, one-shot native send. The UI must select exactly one local
/// source and one OS-pinned peer; this API rechecks the pin and source itself.
/// No background listener, queued sending, Vault access or peer enrollment.
pub async fn native_send_one(
    peer: String,
    source: PathBuf,
    listening: impl FnOnce(Vec<SocketAddr>),
) -> Result<(), String> {
    parse_peer(&peer)?;
    if !source.is_absolute() {
        return Err("select an absolute source file".into());
    }
    run_mode(Mode::Send { peer, source }, listening).await
}

/// Explicit WAN sender reuses the same bounded one-file QUIC protocol and
/// protected peer identity; n0 discovery/relay is opt-in, never automatic.
pub async fn native_send_internet_one(
    peer: String,
    source: PathBuf,
    internet_ready: impl FnOnce(Vec<SocketAddr>),
) -> Result<(), String> {
    parse_peer(&peer)?;
    if !source.is_absolute() {
        return Err("select an absolute source file".into());
    }
    run_mode(Mode::SendInternet { peer, source }, internet_ready).await
}

/// Accept only the independently selected QUIC identity. A random peer that
/// happens to connect first must not end the user's one-shot transfer or
/// cause source-file reads. Keep a single global deadline and a small bounded
/// refusal budget; a revoked selected peer still fails immediately.
async fn await_selected_incoming(
    endpoint: &Endpoint,
    expected: EndpointId,
    verify: impl Fn(&iroh::endpoint::Connection) -> Result<(), String>,
) -> Result<iroh::endpoint::Connection, String> {
    const MAX_UNSELECTED: u8 = 8;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let mut unselected = 0u8;
    loop {
        let incoming = tokio::time::timeout_at(deadline, endpoint.accept())
            .await
            .map_err(|_| "selected receiving peer did not connect before timeout")?
            .ok_or("QUIC endpoint stopped")?;
        // Malformed handshakes consume the same bounded rejection budget.
        let attempted = match incoming.accept() {
            Ok(connecting) => {
                let handshake_deadline = std::cmp::min(
                    deadline,
                    tokio::time::Instant::now() + Duration::from_secs(20),
                );
                tokio::time::timeout_at(handshake_deadline, connecting).await
            }
            Err(_) => {
                unselected += 1;
                if unselected >= MAX_UNSELECTED {
                    return Err("too many rejected inbound QUIC attempts".into());
                }
                continue;
            }
        };
        if let Ok(Ok(connection)) = attempted {
            if connection.remote_id() == expected {
                // The expected transport key is not enough: OS pin/revoke
                // must still be freshly checked before admitting this peer.
                if let Err(error) = verify(&connection) {
                    connection.close(0u8.into(), b"selected peer authorization failed");
                    return Err(error);
                }
                return Ok(connection);
            }
            connection.close(0u8.into(), b"unselected peer");
        }
        unselected += 1;
        if unselected >= MAX_UNSELECTED {
            return Err("too many rejected inbound QUIC attempts".into());
        }
    }
}

async fn run_mode(mode: Mode, listening: impl FnOnce(Vec<SocketAddr>)) -> Result<(), String> {
    let internet = matches!(
        &mode,
        Mode::SendInternet { .. } | Mode::ReceiveInbox { address: None, .. }
    );
    let peer = match &mode {
        Mode::Send { peer, .. }
        | Mode::SendInternet { peer, .. }
        | Mode::Receive { peer, .. }
        | Mode::ReceiveInbox { peer, .. } => peer,
    };
    let expected = selected_peer(peer)?;
    // No automatic enrollment, plaintext private key or default-on service.
    let local = with_protected_iroh_secret(|key| key)
        .map_err(|error| format!("OS-backed Akula private key unavailable: {error}"))?;
    if local.public() == expected {
        return Err("cannot pair a device with itself".into());
    }
    let builder = if internet {
        // Explicit Internet action only. Third-party n0 relay sees network
        // metadata but cannot decrypt end-to-end QUIC contents.
        Endpoint::builder(presets::N0)
    } else {
        Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
    };
    let endpoint = builder
        .secret_key(local)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|error| format!("bind selected P2P route: {error}"))?;
    println!("LOCAL_ENDPOINT_ID={}", endpoint.id());
    if internet {
        tokio::time::timeout(Duration::from_secs(60), endpoint.online())
            .await
            .map_err(|_| "n0 discovery/relay not ready within 60 seconds")?;
        println!("INTERNET_DISCOVERY_READY=true THIRD_PARTY_N0=true");
    }
    let result = match mode {
        Mode::Send { peer, source } | Mode::SendInternet { peer, source } => {
            if internet {
                // Empty list means n0 identity-only address discovery is ready.
                // No public or LAN IP is advertised as a guaranteed route.
                listening(Vec::new());
            } else {
                let bound = endpoint.bound_sockets();
                for socket in &bound {
                    println!("LISTEN_UDP={socket} (share reachable LAN IP and UDP port only)");
                }
                // Wildcard 0.0.0.0/:: is not a remote-reachable IP.
                listening(bound);
            }
            let connection =
                await_selected_incoming(&endpoint, expected, |conn| live_peer(conn, &peer)).await?;
            let outcome = tokio::time::timeout(
                Duration::from_secs(120),
                send_one(&connection, &source, || live_peer(&connection, &peer)),
            )
            .await
            .map_err(|_| "file-send operation timed out")?;
            connection.close(0u8.into(), b"one-file beta complete");
            outcome
        }
        Mode::Receive {
            peer,
            address,
            destination,
        } => {
            let addr = EndpointAddr::new(expected).with_ip_addr(address);
            let connection =
                tokio::time::timeout(Duration::from_secs(20), endpoint.connect(addr, ALPN))
                    .await
                    .map_err(|_| "direct LAN connection timed out")?
                    .map_err(|error| format!("authenticated QUIC connection: {error}"))?;
            live_peer(&connection, &peer)?;
            let inbox_root = inbox::default_root()?;
            let outcome = tokio::time::timeout(
                Duration::from_secs(120),
                receive_one(
                    &connection,
                    &destination,
                    &inbox_root,
                    *endpoint.id().as_bytes(),
                    || live_peer(&connection, &peer),
                ),
            )
            .await
            .map_err(|_| "file-receive operation timed out")?;
            // Closing early after an ACK can prevent delivery on some paths;
            // receiver merely reports local verified bytes, never durable ACK.
            if outcome.is_ok() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            connection.close(0u8.into(), b"one-file beta complete");
            outcome
        }
        Mode::ReceiveInbox { peer, address } => {
            let addr = match address {
                Some(lan) => {
                    validate_direct_lan_address(lan)?;
                    EndpointAddr::new(expected).with_ip_addr(lan)
                }
                None => EndpointAddr::new(expected),
            };
            let connect_timeout = if internet {
                Duration::from_secs(90)
            } else {
                Duration::from_secs(20)
            };
            let connection = tokio::time::timeout(connect_timeout, endpoint.connect(addr, ALPN))
                .await
                .map_err(|_| "selected P2P route connection timed out")?
                .map_err(|error| format!("authenticated QUIC connection: {error}"))?;
            live_peer(&connection, &peer)?;
            let inbox_root = inbox::default_root()?;
            let outcome = tokio::time::timeout(
                Duration::from_secs(120),
                receive_one_target(
                    &connection,
                    None,
                    &inbox_root,
                    *endpoint.id().as_bytes(),
                    || live_peer(&connection, &peer),
                ),
            )
            .await
            .map_err(|_| "native Inbox receive timed out")?;
            if outcome.is_ok() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            connection.close(0u8.into(), b"native one-file Inbox complete");
            outcome
        }
    };
    endpoint.close().await;
    result
}

async fn run() -> Result<(), String> {
    let mode = parse_mode(env::args().skip(1))?;
    run_mode(mode, |_| {}).await
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("P2P_ONE_FILE_BETA_ERROR={error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_send_refuses_malformed_peer_before_lan_bind() {
        let source = std::env::temp_dir().join("not-opened-source.txt");
        let mut listening = false;
        let result = native_send_one("not-a-peer".to_owned(), source, |_| {
            listening = true;
        })
        .await;
        assert!(result.is_err());
        assert!(!listening);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_quic_sender_ignores_unselected_peer_and_admits_expected_peer() {
        use std::net::{IpAddr, Ipv4Addr};

        // All three identities are throwaway local test keys; no OS peer
        // records, external n0 discovery or user-file reads are involved.
        let sender = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let actual_receiver = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let stranger = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let selected = actual_receiver.id();
        let sender_id = sender.id();
        let port = sender
            .bound_sockets()
            .into_iter()
            .find(|socket| socket.is_ipv4())
            .expect("an IPv4 UDP socket on Windows/Linux CI")
            .port();
        let destination = EndpointAddr::new(sender.id())
            .with_ip_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
        let sender_task = tokio::spawn(async move {
            let accepted = await_selected_incoming(&sender, selected, |_| Ok(()))
                .await
                .expect("stranger must not terminate the pending transfer");
            assert_eq!(accepted.remote_id(), selected);
            accepted.close(0u8.into(), b"accepted real peer");
            sender.close().await;
        });
        let stranger_connection = stranger.connect(destination.clone(), ALPN).await.unwrap();
        assert_eq!(stranger_connection.remote_id(), sender_id);
        stranger_connection.close(0u8.into(), b"never authorized");
        let confirmed_connection = actual_receiver.connect(destination, ALPN).await.unwrap();
        assert_eq!(confirmed_connection.remote_id(), sender_id);
        tokio::time::timeout(Duration::from_secs(25), sender_task)
            .await
            .unwrap()
            .unwrap();
        stranger.close().await;
        actual_receiver.close().await;
    }

    #[tokio::test]
    async fn native_internet_entrypoints_reject_bad_peer_before_n0_network() {
        let source = std::env::temp_dir().join("never-opened-onefile-internet-test");
        let mut advertised = false;
        assert!(
            native_send_internet_one("invalid".into(), source, |_| advertised = true)
                .await
                .is_err()
        );
        assert!(!advertised);
        assert!(native_receive_internet_one("invalid".into()).await.is_err());
    }

    #[test]
    fn internet_routing_is_never_selected_by_legacy_lan_modes() {
        let id = iroh::SecretKey::generate().public().to_string();
        let legacy = Mode::ReceiveInbox {
            peer: id.clone(),
            address: Some("192.168.1.23:5000".parse().unwrap()),
        };
        let explicitly_internet = Mode::ReceiveInbox {
            peer: id,
            address: None,
        };
        assert!(!matches!(legacy, Mode::ReceiveInbox { address: None, .. }));
        assert!(matches!(
            explicitly_internet,
            Mode::ReceiveInbox { address: None, .. }
        ));
    }

    #[test]
    fn native_address_accepts_private_lan_and_rejects_public_or_wildcard() {
        assert!(validate_direct_lan_address("192.168.1.50:33099".parse().unwrap()).is_ok());
        assert!(validate_direct_lan_address("127.0.0.1:33099".parse().unwrap()).is_ok());
        assert!(validate_direct_lan_address("8.8.8.8:33099".parse().unwrap()).is_err());
        assert!(validate_direct_lan_address("0.0.0.0:33099".parse().unwrap()).is_err());
        assert!(validate_direct_lan_address("192.168.1.50:0".parse().unwrap()).is_err());
    }

    #[tokio::test]
    async fn native_receive_rejects_unverified_peer_before_any_network_use() {
        let result = native_receive_one(
            "not-an-enrolled-key".into(),
            "192.168.1.20:45678".parse().unwrap(),
        )
        .await;
        assert!(result.is_err());
    }

    #[test]
    fn strict_explicit_cli_modes() {
        let id = iroh::SecretKey::generate().public().to_string();
        let file = std::env::temp_dir().join("file.bin");
        let path = file.to_string_lossy().to_string();
        assert!(matches!(
            parse_mode(["send".into(), id.clone(), path.clone()]),
            Ok(Mode::Send { .. })
        ));
        assert!(parse_mode(["send".into(), id.clone(), "relative.txt".into()]).is_err());
        assert!(parse_mode(["send".into(), "f".repeat(63), path.clone()]).is_err());
        assert!(parse_mode(["send".into(), "0".repeat(64), path.clone()]).is_err());
        assert!(parse_mode(["receive".into(), id.clone(), "host:5".into(), path.clone()]).is_err());
        assert!(
            parse_mode([
                "receive".into(),
                id.clone(),
                "127.0.0.1:0".into(),
                path.clone()
            ])
            .is_err()
        );
        assert!(
            parse_mode([
                "receive".into(),
                id.clone(),
                "127.0.0.1:1024".into(),
                "relative".into()
            ])
            .is_err()
        );
        assert!(parse_mode(["send".into(), id, path, "extra".into()]).is_err());
    }

    #[test]
    fn exact_frame_has_length_and_digest_without_any_remote_path() {
        let bytes = b"one verified P2P file";
        let frame = frame_for(bytes);
        let (size, root, data) = decode_frame(&frame).unwrap();
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(root, *blake3::hash(bytes).as_bytes());
        assert_eq!(data, bytes);
        let mut forged = frame.clone();
        forged[30] ^= 7;
        assert!(decode_frame(&forged).is_err());
        let mut extra = frame.clone();
        extra.push(0);
        assert!(decode_frame(&extra).is_err());
        assert!(decode_frame(&frame[..frame.len() - 1]).is_err());
        assert!(decode_frame(&[]).is_err());
    }

    #[test]
    fn received_file_is_create_new_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("user-selected.bin");
        let bytes = b"one directly authenticated file";
        let frame = frame_for(bytes);
        save_received(&frame, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        assert!(save_received(&frame, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        let bad = frame_for(b"different");
        let other = dir.path().join("never.bin");
        let mut corrupt = bad.clone();
        corrupt[16] ^= 1;
        assert!(save_received(&corrupt, &other).is_err());
        assert!(!other.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_iroh_quic_one_file_roundtrip_without_user_credentials() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("selected-source.txt");
        let output = dir.path().join("receiver-selected-name.txt");
        let inbox_root = dir.path().join("inbox");
        let bytes = b"Akula P2P one-file LAN beta real QUIC payload";
        fs::write(&source, bytes).unwrap();

        // These disposable TLS keys are test-only. Real production CLI always
        // loads protected OS credentials and checks durable peer pins first.
        let sender = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let sender_id = sender.id();
        let socket = sender
            .bound_sockets()
            .into_iter()
            .find(|socket| socket.is_ipv4())
            .expect("IPv4 loopback is required on Windows/Linux CI");
        let direct = EndpointAddr::new(sender_id).with_ip_addr(SocketAddr::new(
            match socket {
                SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
                SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
            },
            socket.port(),
        ));
        let receiver = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let receiver_id = receiver.id();

        let sender_task = tokio::spawn(async move {
            let incoming = tokio::time::timeout(Duration::from_secs(25), sender.accept())
                .await
                .unwrap()
                .unwrap();
            let connection = incoming.accept().unwrap().await.unwrap();
            assert_eq!(connection.remote_id(), receiver_id);
            tokio::time::timeout(
                Duration::from_secs(25),
                send_one(&connection, &source, || Ok(())),
            )
            .await
            .unwrap()
            .unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            connection.close(0u8.into(), b"test transfer finished");
            sender.close().await;
        });

        let connection =
            tokio::time::timeout(Duration::from_secs(25), receiver.connect(direct, ALPN))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(connection.remote_id(), sender_id);
        tokio::time::timeout(
            Duration::from_secs(25),
            receive_one(
                &connection,
                &output,
                &inbox_root,
                *receiver_id.as_bytes(),
                || Ok(()),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        connection.close(0u8.into(), b"test complete");
        receiver.close().await;
        sender_task.await.unwrap();
        assert_eq!(fs::read(&output).unwrap(), bytes);
        // The same actual authenticated QUIC bytes must survive a closed and
        // reopened receiver journal, independently of the user-selected copy.
        let cold = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
        let items = cold.list().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].1,
            akula_inbox_ledger_lab::LocalStatus::VerifiedLocal
        );
        assert_eq!(items[0].0.sender_key, *sender_id.as_bytes());
        assert_eq!(items[0].0.recipient_key, *receiver_id.as_bytes());
        assert_eq!(fs::read(cold.object_path(items[0].0.id)).unwrap(), bytes);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_iroh_native_received_inbox_only_roundtrip_without_user_credentials() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("selected-source.txt");
        let inbox_root = dir.path().join("inbox");
        fs::create_dir(&inbox_root).unwrap();
        let bytes = b"Akula P2P one-file LAN beta real QUIC payload";
        fs::write(&source, bytes).unwrap();

        // These disposable TLS keys are test-only. Real production CLI always
        // loads protected OS credentials and checks durable peer pins first.
        let sender = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let sender_id = sender.id();
        let socket = sender
            .bound_sockets()
            .into_iter()
            .find(|socket| socket.is_ipv4())
            .expect("IPv4 loopback is required on Windows/Linux CI");
        let direct = EndpointAddr::new(sender_id).with_ip_addr(SocketAddr::new(
            match socket {
                SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
                SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
            },
            socket.port(),
        ));
        let receiver = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let receiver_id = receiver.id();

        let sender_task = tokio::spawn(async move {
            let incoming = tokio::time::timeout(Duration::from_secs(25), sender.accept())
                .await
                .unwrap()
                .unwrap();
            let connection = incoming.accept().unwrap().await.unwrap();
            assert_eq!(connection.remote_id(), receiver_id);
            tokio::time::timeout(
                Duration::from_secs(25),
                send_one(&connection, &source, || Ok(())),
            )
            .await
            .unwrap()
            .unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            connection.close(0u8.into(), b"test transfer finished");
            sender.close().await;
        });

        let connection =
            tokio::time::timeout(Duration::from_secs(25), receiver.connect(direct, ALPN))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(connection.remote_id(), sender_id);
        tokio::time::timeout(
            Duration::from_secs(25),
            receive_one_target(
                &connection,
                None,
                &inbox_root,
                *receiver_id.as_bytes(),
                || Ok(()),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        connection.close(0u8.into(), b"test complete");
        receiver.close().await;
        sender_task.await.unwrap();
        // The same actual authenticated QUIC bytes must survive a closed and
        // reopened permanent receiver journal, without any exported copy.
        let cold = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
        let items = cold.list().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].1,
            akula_inbox_ledger_lab::LocalStatus::VerifiedLocal
        );
        assert_eq!(items[0].0.sender_key, *sender_id.as_bytes());
        assert_eq!(items[0].0.recipient_key, *receiver_id.as_bytes());
        assert_eq!(fs::read(cold.object_path(items[0].0.id)).unwrap(), bytes);
        // No user-selected export exists: only source + private Inbox remain.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
