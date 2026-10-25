//! Akula Library research tool: explicit two-process, direct-UDP transfer only.
//! NO discovery, NO public relay, NO production File Gate integration.
use akula_library_iroh_lab::{Book, QuotaState, Server, direct_endpoint, get_parts_with_usage, manifest_from_with_usage, WireUsage, durable::DurableDownload, persistent_quota::PersistentQuota, rate_limit::RateGate};
use anyhow::{Context, Result, bail, ensure};
use iroh::{EndpointAddr, EndpointId, SecretKey};
use std::{collections::{HashMap, HashSet}, env, fs, io::{Read, Write}, net::SocketAddr, path::Path, str::FromStr};

fn parse_args() -> Result<(String, HashMap<String, String>)> {
    let mut args = env::args().skip(1);
    let command = args.next().context("usage: identity|serve|fetch --key PATH ...")?;
    let mut flags = HashMap::new();
    while let Some(key) = args.next() {
        ensure!(key.starts_with("--"), "expected --flag, got {key}");
        let value = args.next().context("missing value for flag")?;
        ensure!(!value.starts_with("--"), "missing value for {key}");
        ensure!(flags.insert(key[2..].to_string(), value).is_none(), "repeated flag");
    }
    Ok((command, flags))
}
fn take(m: &mut HashMap<String, String>, name: &str) -> Result<String> {
    m.remove(name).with_context(|| format!("missing --{name}"))
}
fn take_or(m: &mut HashMap<String, String>, name: &str, default: &str) -> String {
    m.remove(name).unwrap_or_else(|| default.to_string())
}
fn assert_no_flags(m: &HashMap<String, String>) -> Result<()> {
    ensure!(m.is_empty(), "unsupported flags: {:?}", m.keys().collect::<Vec<_>>());
    Ok(())
}
fn read_key(path: &Path) -> Result<SecretKey> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(meta.file_type().is_file(), "key must be a regular file, not a symlink");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        ensure!(meta.permissions().mode() & 0o077 == 0, "key file must be mode 0600 (chmod 600)");
    }
    ensure!(meta.len() == 32, "key file must contain exactly 32 bytes");
    let bytes = fs::read(path)?;
    Ok(SecretKey::from_bytes(&bytes.try_into().map_err(|_| anyhow::anyhow!("invalid key length"))?))
}
fn init_key(path: &Path) -> Result<SecretKey> {
    if fs::symlink_metadata(path).is_ok() { return read_key(path); }
    let key = SecretKey::generate();
    let mut opts = fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut out = opts.open(path)?;
    out.write_all(&key.to_bytes())?;
    out.sync_all()?;
    Ok(key)
}
fn read_book(path: &Path) -> Result<Book> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(meta.file_type().is_file(), "book must be a regular file");
    ensure!(meta.len() <= akula_library_iroh_lab::MAX_BOOK_BYTES as u64, "book too large");
    let mut src = fs::File::open(path)?;
    let mut bytes = vec![];
    std::io::Read::by_ref(&mut src).take((akula_library_iroh_lab::MAX_BOOK_BYTES+1) as u64).read_to_end(&mut bytes)?;
    Book::new(bytes)
}
fn hex_digest(bytes: &[u8;32]) -> String { bytes.iter().map(|v| format!("{v:02x}")).collect() }

#[tokio::main]
async fn main() -> Result<()> {
    let (command, mut flags) = parse_args()?;
    match command.as_str() {
        "identity" => {
            let path = take(&mut flags, "key")?;
            assert_no_flags(&flags)?;
            let id = init_key(Path::new(&path))?.public();
            println!("IDENTITY {id}");
        }
        "serve" => {
            let key = read_key(Path::new(&take(&mut flags, "key")?))?;
            let book = read_book(Path::new(&take(&mut flags, "book")?))?;
            let receiver = EndpointId::from_str(&take(&mut flags, "allow")?)?;
            let quota_dir = take(&mut flags, "quota-state")?;
            let bind_addr: SocketAddr = take_or(&mut flags, "bind", "127.0.0.1:0").parse()?;
            let global_quota: u64 = take_or(&mut flags, "global-quota", "67108864").parse()?;
            let peer_quota: u64 = take_or(&mut flags, "peer-quota", "33554432").parse()?;
            let global_rate: u64 = take_or(&mut flags, "global-rate", "4194304").parse()?;
            let peer_rate: u64 = take_or(&mut flags, "peer-rate", "2097152").parse()?;
            assert_no_flags(&flags)?;
            // Open and lock the journal before allowing any connections.
            let rate = RateGate::new(global_rate, peer_rate)?;
            let quota = PersistentQuota::open(Path::new(&quota_dir), key.public(), global_quota, peer_quota)?;
            let endpoint = direct_endpoint(key, bind_addr).await?;
            let sock = endpoint.bound_sockets().into_iter().find(|s| s.is_ipv4()).context("IPv4 bind failed")?;
            // The listener may bind 0.0.0.0; for another host the operator must use the host's routable IP.
            println!("READY id={} addr={} hash={}", endpoint.id(), sock, hex_digest(&book.manifest.digest));
            std::io::stdout().flush()?;
            let server = Server::new(book, HashSet::from([receiver]), QuotaState::new(global_quota, peer_quota)).with_persistent_quota(quota).with_rate_limit(rate);
            server.run(endpoint).await;
        }
        "fetch" => {
            let key = read_key(Path::new(&take(&mut flags, "key")?))?;
            let remote_id = EndpointId::from_str(&take(&mut flags, "remote-id")?)?;
            let remote_addr: SocketAddr = take(&mut flags, "remote-addr")?.parse()?;
            ensure!(!remote_addr.ip().is_unspecified(), "remote address must be reachable, not 0.0.0.0");
            let stage = take(&mut flags, "stage")?;
            let output = take(&mut flags, "output")?;
            let max_parts: usize = take_or(&mut flags, "max-parts", "64").parse()?;
            // Test-only explicit source address; 127/8 aliases can emulate independent
            // ingress IPs on one Linux runner without external traffic.
            let client_bind: SocketAddr = take_or(&mut flags, "bind", "127.0.0.1:0").parse()?;
            ensure!(max_parts <= akula_library_iroh_lab::MAX_CHUNKS, "max-parts too high");
            assert_no_flags(&flags)?;
            let endpoint = direct_endpoint(key, client_bind).await?;
            let remote = EndpointAddr::new(remote_id).with_ip_addr(remote_addr);
            let (manifest, first_usage) = manifest_from_with_usage(&endpoint, remote.clone()).await?;
            let mut wire = WireUsage::default();
            wire.add(first_usage);
            let mut durable = DurableDownload::open(Path::new(&stage), manifest)?;
            let mut received = 0usize;
            while received < max_parts {
                let Some(idx) = durable.first_missing() else { break };
                let (n, usage) = get_parts_with_usage(&endpoint, remote.clone(), &mut durable.partial, Some(1)).await?;
                wire.add(usage);
                ensure!(n == 1 && durable.partial.verified_part(idx).is_some(), "missing requested part");
                durable.commit_verified_part(idx)?;
                received += 1;
            }
            if durable.partial.is_complete() {
                durable.finish_to(Path::new(&output))?;
                println!("COMPLETE bytes={} chunks={} new={} hash={}",
                    durable.partial.manifest.len, durable.partial.manifest.parts.len(), received,
                    hex_digest(&durable.partial.manifest.digest));
            } else {
                println!("PARTIAL verified={} total={} new={}", durable.partial.verified_count(),
                    durable.partial.manifest.parts.len(), received);
            }
            println!("CLIENT_WIRE_METRICS udp_tx_bytes={} udp_rx_bytes={} udp_tx_datagrams={} udp_rx_datagrams={} connections={} payload_verified={} udp_rx_to_payload_ratio={:.5}",
                wire.udp_tx_bytes, wire.udp_rx_bytes, wire.udp_tx_datagrams,
                wire.udp_rx_datagrams, wire.connections,
                durable.partial.manifest.len,
                (wire.udp_rx_bytes as f64) / (durable.partial.manifest.len.max(1) as f64));
            endpoint.close().await;
        }
        _ => bail!("unknown command; use identity|serve|fetch"),
    }
    Ok(())
}