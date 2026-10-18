//! Isolated Akula Library research experiment. Not production, no WAN/relay discovery.
//! Iroh 1.3.0 QUIC authenticates peer endpoint IDs. Application access is an allowlist.
//! Explicit quotas below bound file payload bytes, NOT actual wire bytes including QUIC/IP.

use anyhow::{bail, ensure, Context, Result};
pub mod durable;
pub mod persistent_quota;
pub mod rate_limit;
pub mod preauth;
use iroh::{endpoint::{presets, Connection, IncomingAddr}, Endpoint, EndpointAddr, EndpointId, SecretKey};
use std::{collections::{HashMap, HashSet}, net::SocketAddr, sync::{Arc, Mutex}};
use tokio::{sync::Semaphore, time::{timeout, Duration}};

pub const ALPN: &[u8] = b"akula-library/iroh-lab/1";
pub const CHUNK_SIZE: usize = 256 * 1024;
pub const MAX_BOOK_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHUNKS: usize = MAX_BOOK_BYTES / CHUNK_SIZE;
const MANIFEST_MAGIC: &[u8; 4] = b"AKM1";
const MAX_MANIFEST_BYTES: usize = 4 + 8 + 4 + 4 + 32 + MAX_CHUNKS * 32;
const REQUEST_LIMIT: usize = 1 + 32 + 4;
pub const MAX_ACTIVE_CONNECTIONS: usize = 32; // bounded lab admission, even during overload
pub const MAX_CONNECTIONS_PER_PEER: usize = 2;
/// All endpoint keys assigned to one owner-controlled principal share this cap.
pub const MAX_CONNECTIONS_PER_PRINCIPAL: usize = 8;
const MAX_STREAMS_PER_CONNECTION: usize = 128;
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const OP_MANIFEST: u8 = 1;
const OP_CHUNK: u8 = 2;
const OK: u8 = 0;
const BAD_REQUEST: u8 = 1;
const QUOTA: u8 = 2;
const NOT_FOUND: u8 = 3;

pub(crate) fn sha(bytes: &[u8]) -> [u8; 32] { *blake3::hash(bytes).as_bytes() }

/// UDP datagram payload statistics observed on *one* QUIC connection using
/// the pinned Roha/noq forks. NOT IP/UDP headers, relay overhead, other
/// processes, or an enforceable interface-level traffic budget.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireUsage {
    pub udp_tx_bytes: u64,
    pub udp_rx_bytes: u64,
    pub udp_tx_datagrams: u64,
    pub udp_rx_datagrams: u64,
    pub connections: u64,
}
impl WireUsage {
    pub fn sample(conn: &Connection) -> Self {
        let s = conn.stats();
        Self { udp_tx_bytes: s.udp_tx.bytes, udp_rx_bytes: s.udp_rx.bytes,
               udp_tx_datagrams: s.udp_tx.datagrams,
               udp_rx_datagrams: s.udp_rx.datagrams, connections: 1 }
    }
    pub fn add(&mut self, rhs: Self) {
        self.udp_tx_bytes = self.udp_tx_bytes.saturating_add(rhs.udp_tx_bytes);
        self.udp_rx_bytes = self.udp_rx_bytes.saturating_add(rhs.udp_rx_bytes);
        self.udp_tx_datagrams = self.udp_tx_datagrams.saturating_add(rhs.udp_tx_datagrams);
        self.udp_rx_datagrams = self.udp_rx_datagrams.saturating_add(rhs.udp_rx_datagrams);
        self.connections = self.connections.saturating_add(rhs.connections);
    }
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub len: u64,
    pub digest: [u8; 32],
    pub parts: Vec<[u8; 32]>,
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.len <= MAX_BOOK_BYTES as u64, "book exceeds cap");
        let required = self.len.div_ceil(CHUNK_SIZE as u64) as usize;
        ensure!(self.parts.len() == required && required <= MAX_CHUNKS, "invalid chunk count");
        Ok(())
    }
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(52 + self.parts.len() * 32);
        out.extend(MANIFEST_MAGIC);
        out.extend(self.len.to_be_bytes());
        out.extend((CHUNK_SIZE as u32).to_be_bytes());
        out.extend((self.parts.len() as u32).to_be_bytes());
        out.extend(self.digest);
        for part in &self.parts { out.extend(part); }
        out
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() >= 52 && bytes.len() <= MAX_MANIFEST_BYTES, "invalid manifest size");
        ensure!(&bytes[..4] == MANIFEST_MAGIC, "invalid manifest magic");
        let len = u64::from_be_bytes(bytes[4..12].try_into()?);
        let part_bytes = u32::from_be_bytes(bytes[12..16].try_into()?) as usize;
        ensure!(part_bytes == CHUNK_SIZE, "unsupported part size");
        let count = u32::from_be_bytes(bytes[16..20].try_into()?) as usize;
        ensure!(count <= MAX_CHUNKS && bytes.len() == 52 + count * 32, "invalid manifest count");
        let digest = bytes[20..52].try_into()?;
        let parts = bytes[52..].chunks_exact(32).map(|s| s.try_into().expect("fixed-size slices")).collect();
        let manifest = Self { len, digest, parts };
        manifest.validate()?;
        Ok(manifest)
    }
    pub fn part_len(&self, index: usize) -> Result<usize> {
        ensure!(index < self.parts.len(), "part out of range");
        Ok(((self.len as usize) - index * CHUNK_SIZE).min(CHUNK_SIZE))
    }
}

#[derive(Clone)]
pub struct Book {
    bytes: Arc<Vec<u8>>,
    pub manifest: Manifest,
}

impl Book {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BOOK_BYTES, "book too large");
        let digest = sha(&bytes);
        let parts = bytes.chunks(CHUNK_SIZE).map(sha).collect();
        let manifest = Manifest {len: bytes.len() as u64, digest, parts};
        manifest.validate()?;
        Ok(Self {bytes: Arc::new(bytes), manifest})
    }
    pub(crate) fn part(&self, i: usize) -> Result<&[u8]> {
        let len = self.manifest.part_len(i)?;
        Ok(&self.bytes[i*CHUNK_SIZE..i*CHUNK_SIZE+len])
    }
}

/// Shared charging policy. Reservation is never refunded on aborted sends (fail-closed).
#[derive(Debug)]
pub struct QuotaState {
    global_limit: u64,
    per_peer_limit: u64,
    global_used: u64,
    per_peer_used: HashMap<EndpointId, u64>,
}
impl QuotaState {
    pub fn new(global_limit: u64, per_peer_limit: u64) -> Self {
        Self { global_limit, per_peer_limit, global_used: 0, per_peer_used: HashMap::new() }
    }
    pub fn used(&self) -> u64 { self.global_used }
    pub fn used_for(&self, id: &EndpointId) -> u64 { *self.per_peer_used.get(id).unwrap_or(&0) }
    fn reserve(&mut self, id: EndpointId, bytes: usize) -> bool {
        let n = bytes as u64;
        let Some(global) = self.global_used.checked_add(n) else {return false};
        let old = self.used_for(&id);
        let Some(peer) = old.checked_add(n) else {return false};
        if global > self.global_limit || peer > self.per_peer_limit { return false; }
        self.global_used = global;
        self.per_peer_used.insert(id, peer);
        true
    }
}

/// Trusted principal is a stable, owner-assigned identity. A remote peer never
/// gets to choose or assert a principal in its network request.
#[derive(Default)]
struct AdmissionCounts {
    per_identity: HashMap<EndpointId, usize>,
    per_principal: HashMap<EndpointId, usize>,
}
/// RAII lease: the identity and principal counters change atomically under one
/// lock, even when a QUIC task is cancelled or fails before it sends any data.
struct ActivePeerLease {
    counts: Arc<Mutex<AdmissionCounts>>,
    id: EndpointId,
    principal: EndpointId,
}
impl Drop for ActivePeerLease {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(n) = counts.per_identity.get_mut(&self.id) {
            *n -= 1;
            if *n == 0 { counts.per_identity.remove(&self.id); }
        }
        if let Some(n) = counts.per_principal.get_mut(&self.principal) {
            *n -= 1;
            if *n == 0 { counts.per_principal.remove(&self.principal); }
        }
    }
}

#[derive(Clone)]
pub struct Server {
    book: Book,
    allowed: Arc<HashSet<EndpointId>>,
    quota: Arc<Mutex<QuotaState>>,
    disk_quota: Option<Arc<Mutex<persistent_quota::PersistentQuota>>>,
    rate: Option<Arc<rate_limit::RateGate>>,
    active: Arc<Mutex<AdmissionCounts>>,
    /// Every allowed endpoint key has a stable trusted principal. By default
    /// each key is its own principal; group assignment is opt-in by the owner.
    principal_for: Arc<HashMap<EndpointId, EndpointId>>,
    idle_timeout: Duration,
    request_timeout: Duration,
}
impl Server {
    pub fn new(book: Book, allowed: HashSet<EndpointId>, limits: QuotaState) -> Self {
        let principal_for = allowed.iter().map(|id| (*id, *id)).collect();
        Self {
            book, allowed: Arc::new(allowed),
            quota: Arc::new(Mutex::new(limits)), disk_quota: None, rate: None,
            active: Arc::new(Mutex::new(AdmissionCounts::default())),
            principal_for: Arc::new(principal_for),
            idle_timeout: DEFAULT_IDLE_TIMEOUT, request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
    pub fn quota(&self) -> Arc<Mutex<QuotaState>> { self.quota.clone() }
    pub fn with_persistent_quota(mut self, quota: persistent_quota::PersistentQuota) -> Self {
        self.disk_quota = Some(Arc::new(Mutex::new(quota)));
        self
    }
    pub fn with_rate_limit(mut self, gate: rate_limit::RateGate) -> Self {
        self.rate = Some(Arc::new(gate));
        self
    }
    /// Group rotated/multiple EndpointIds under a locally trusted *stable*
    /// principal key. Mapping is configured by the owner, NEVER by the peer.
    /// Every allowed ID must be mapped; every canonical principal maps to itself.
    /// Incomplete or inconsistent assignments fail closed.
    pub fn with_trusted_principals(mut self,
        mapping: HashMap<EndpointId, EndpointId>) -> Result<Self> {
        ensure!(mapping.len() == self.allowed.len(), "principal mapping must be complete");
        for id in self.allowed.iter() {
            let principal = mapping.get(id).context("missing principal for allowed identity")?;
            ensure!(self.allowed.contains(principal), "principal must be an allowed identity");
            ensure!(mapping.get(principal) == Some(principal),
                "principal key must map to itself");
        }
        self.principal_for = Arc::new(mapping);
        Ok(self)
    }
    /// Shorter timeouts are used only by negative laboratory tests.
    pub fn with_request_timeouts(mut self, idle: Duration, request: Duration) -> Self {
        assert!(!idle.is_zero() && !request.is_zero());
        self.idle_timeout = idle;
        self.request_timeout = request;
        self
    }
    fn admit_peer(&self, peer: EndpointId) -> Option<ActivePeerLease> {
        let principal = *self.principal_for.get(&peer)?;
        let mut state = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if state.per_identity.get(&peer).copied().unwrap_or(0) >= MAX_CONNECTIONS_PER_PEER
            || state.per_principal.get(&principal).copied().unwrap_or(0) >= MAX_CONNECTIONS_PER_PRINCIPAL {
            return None;
        }
        *state.per_identity.entry(peer).or_default() += 1;
        *state.per_principal.entry(principal).or_default() += 1;
        Some(ActivePeerLease { counts: self.active.clone(), id: peer, principal })
    }
    /// Admission on Incoming: no TLS handshake is awaited until an IP budget,
    /// address validation and a bounded handshake slot have been considered.
    pub async fn run(self, endpoint: Endpoint) {
        let slots = Arc::new(Semaphore::new(MAX_ACTIVE_CONNECTIONS));
        let mut preauth = preauth::PreAuthGate::new(std::time::Instant::now());
        loop {
            let Some(incoming) = endpoint.accept().await else { break };
            let peer_ip = match incoming.remote_addr() {
                IncomingAddr::Ip(addr) => addr.ip(),
                _ => {
                    // The isolated direct-UDP lab does not expose relays/custom
                    // transports; future transports require their own policy.
                    incoming.ignore();
                    continue;
                }
            };
            if !preauth.admit(peer_ip, std::time::Instant::now()) {
                incoming.ignore();
                continue;
            }
            let Ok(permit) = slots.clone().try_acquire_owned() else {
                preauth.no_slot();
                incoming.ignore();
                continue;
            };
            if !incoming.remote_addr_validated() {
                // QUIC address validation is a separate pre-handshake round-trip.
                // It does not verify an EndpointId or grant application ACL access.
                preauth.retry();
                if incoming.retry().is_err() {
                    continue;
                }
                continue;
            }
            preauth.started();
            let server = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                match timeout(HANDSHAKE_TIMEOUT, incoming).await {
                    Ok(Ok(conn)) => { let _ = server.handle_connection(conn).await; },
                    _ => (),
                }
            });
        }
    }
    async fn handle_connection(&self, conn: Connection) -> Result<()> {
        let peer = conn.remote_id();
        let principal = self.principal_for.get(&peer).copied();
        let result = if !self.allowed.contains(&peer) || principal.is_none() {
            conn.close(403u32.into(), b"denied");
            Ok(())
        } else if let Some(_lease) = self.admit_peer(peer) {
            self.handle_connection_inner(&conn, principal.expect("validated principal")).await
        } else {
            conn.close(429u32.into(), b"peer/principal connection cap");
            Ok(())
        };
        let usage = WireUsage::sample(&conn);
        println!("SERVER_WIRE_METRICS peer={peer} udp_tx_bytes={} udp_rx_bytes={} udp_tx_datagrams={} udp_rx_datagrams={}",
            usage.udp_tx_bytes, usage.udp_rx_bytes, usage.udp_tx_datagrams, usage.udp_rx_datagrams);
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        result
    }
    async fn handle_connection_inner(&self, conn: &Connection, principal: EndpointId) -> Result<()> {
        for _ in 0..MAX_STREAMS_PER_CONNECTION {
            // An authenticated peer must not occupy a global connection slot forever.
            let (mut send, mut recv) = match timeout(self.idle_timeout, conn.accept_bi()).await {
                Ok(Ok(stream)) => stream,
                Ok(Err(_)) => return Ok(()), // Peer closed the connection.
                Err(_) => {
                    conn.close(408u32.into(), b"idle connection");
                    return Ok(());
                }
            };
            // Request upload is tiny (at most 37 bytes); it must be completed
            // promptly even when book downloads themselves are rate-limited.
            let incoming = timeout(self.request_timeout, recv.read_to_end(REQUEST_LIMIT)).await;
            let response: Vec<u8> = match incoming {
                Ok(Ok(raw)) => self.reply(principal, &raw),
                Ok(Err(_)) => vec![BAD_REQUEST],
                Err(_) => {
                    conn.close(408u32.into(), b"incomplete request");
                    return Ok(());
                }
            };
            if let Some(gate) = &self.rate {
                // Pacing applies to all application response bytes including manifest/status.
                // The persistent ledger separately accounts only accepted file payload.
                for piece in response.chunks(rate_limit::PACE_SLICE) {
                    gate.wait(principal, piece.len()).await?;
                    timeout(RESPONSE_TIMEOUT, send.write_all(piece)).await??;
                }
            } else {
                timeout(RESPONSE_TIMEOUT, send.write_all(&response)).await??;
            }
            send.finish()?;
        }
        conn.close(429u32.into(), b"stream count limit");
        Ok(())
    }
    fn reply(&self, peer: EndpointId, req: &[u8]) -> Vec<u8> {
        if req == [OP_MANIFEST] {
            let mut out = vec![OK];
            out.extend(self.book.manifest.encode());
            return out;
        }
        if req.first() != Some(&OP_CHUNK) || req.len() != REQUEST_LIMIT { return vec![BAD_REQUEST]; }
        if req[1..33] != self.book.manifest.digest[..] { return vec![NOT_FOUND]; }
        let index = u32::from_be_bytes(req[33..37].try_into().expect("validated request length")) as usize;
        let Ok(part) = self.book.part(index) else { return vec![NOT_FOUND] };
        let permitted = if let Some(disk) = &self.disk_quota {
            match disk.lock().expect("poisoned persistent quota mutex").reserve(peer, part.len()) {
                Ok(permitted) => permitted,
                Err(_) => return vec![QUOTA], // fail closed on storage error
            }
        } else {
            self.quota.lock().expect("poisoned quota mutex").reserve(peer, part.len())
        };
        if !permitted { return vec![QUOTA] }
        let mut out = Vec::with_capacity(part.len()+1);
        out.push(OK);
        out.extend(part);
        out
    }
}

pub async fn private_endpoint(secret: Option<SecretKey>) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")?
        .alpns(vec![ALPN.to_vec()]);
    if let Some(secret) = secret { builder = builder.secret_key(secret); }
    Ok(builder.bind().await?)
}

/// Explicit direct-only UDP endpoint for the two-process LAN/WAN laboratory.
/// It does not use an external relay or the N0 address lookup service.
pub async fn direct_endpoint(secret: SecretKey, bind_addr: SocketAddr) -> Result<Endpoint> {
    Ok(Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr(bind_addr)?
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .bind().await?)
}

pub fn loopback_addr(endpoint: &Endpoint) -> Result<EndpointAddr> {
    let addr: SocketAddr = endpoint.bound_sockets().into_iter().find(|addr| addr.is_ipv4())
        .context("missing IPv4 socket")?;
    Ok(EndpointAddr::new(endpoint.id()).with_ip_addr(addr))
}

pub struct PartialDownload {
    pub manifest: Manifest,
    parts: Vec<Option<Vec<u8>>>,
}
impl PartialDownload {
    pub fn new(manifest: Manifest) -> Result<Self> {
        manifest.validate()?;
        Ok(Self { parts: vec![None; manifest.parts.len()], manifest })
    }
    pub fn verified_part(&self, index: usize) -> Option<&[u8]> { self.parts.get(index)?.as_deref() }
    pub fn verified_count(&self) -> usize { self.parts.iter().filter(|part| part.is_some()).count() }
    pub fn is_complete(&self) -> bool { self.verified_count() == self.parts.len() }
    pub fn add_part(&mut self, index: usize, bytes: Vec<u8>) -> Result<()> {
        ensure!(self.manifest.part_len(index)? == bytes.len(), "invalid part length");
        ensure!(sha(&bytes) == self.manifest.parts[index], "invalid part hash");
        self.parts[index] = Some(bytes);
        Ok(())
    }
    pub fn finish(self) -> Result<Vec<u8>> {
        ensure!(self.is_complete(), "download incomplete");
        let mut bytes = Vec::with_capacity(self.manifest.len as usize);
        for part in self.parts { bytes.extend(part.context("missing part")?); }
        ensure!(bytes.len() as u64 == self.manifest.len && sha(&bytes) == self.manifest.digest, "whole-book hash mismatch");
        Ok(bytes)
    }
}

async fn ask(conn: &Connection, req: &[u8], max_reply: usize) -> Result<Vec<u8>> {
    let (mut send, mut recv) = timeout(RESPONSE_TIMEOUT, conn.open_bi()).await??;
    timeout(RESPONSE_TIMEOUT, send.write_all(req)).await??;
    send.finish()?;
    let reply = timeout(RESPONSE_TIMEOUT, recv.read_to_end(max_reply + 1)).await??;
    ensure!(!reply.is_empty(), "empty reply");
    if reply[0] != OK {
        bail!("server denied request, status {}", reply[0]);
    }
    Ok(reply[1..].to_vec())
}

/// One authenticated connection; every request is bounded and its payload verified.
/// Return a verified manifest and a best-effort UDP byte snapshot for this connection.
pub async fn manifest_from_with_usage(endpoint: &Endpoint, remote: EndpointAddr) -> Result<(Manifest, WireUsage)> {
    let conn = timeout(CONNECT_TIMEOUT, endpoint.connect(remote, ALPN)).await??;
    let payload = ask(&conn, &[OP_MANIFEST], MAX_MANIFEST_BYTES).await?;
    let manifest = Manifest::decode(&payload)?;
    let usage = WireUsage::sample(&conn);
    conn.close(0u32.into(), b"done");
    Ok((manifest, usage))
}
pub async fn manifest_from(endpoint: &Endpoint, remote: EndpointAddr) -> Result<Manifest> {
    Ok(manifest_from_with_usage(endpoint, remote).await?.0)
}

/// Resume verified missing parts, including an observation of this connection's
/// UDP traffic (not a full-host network or billable-byte counter).
pub async fn get_parts_with_usage(endpoint: &Endpoint, remote: EndpointAddr,
    partial: &mut PartialDownload, max_parts: Option<usize>) -> Result<(usize, WireUsage)> {
    let conn = timeout(CONNECT_TIMEOUT, endpoint.connect(remote, ALPN)).await??;
    let remote_manifest = Manifest::decode(&ask(&conn, &[OP_MANIFEST], MAX_MANIFEST_BYTES).await?)?;
    ensure!(remote_manifest == partial.manifest, "remote manifest changed");
    let mut count = 0;
    for index in 0..partial.parts.len() {
        if partial.parts[index].is_some() { continue; }
        if count >= max_parts.unwrap_or(usize::MAX) { break; }
        let mut req = vec![OP_CHUNK];
        req.extend(partial.manifest.digest);
        req.extend((index as u32).to_be_bytes());
        let bytes = ask(&conn, &req, CHUNK_SIZE).await?;
        partial.add_part(index, bytes)?;
        count += 1;
    }
    let usage = WireUsage::sample(&conn);
    conn.close(0u32.into(), b"done");
    Ok((count, usage))
}
pub async fn get_parts(endpoint: &Endpoint, remote: EndpointAddr,
    partial: &mut PartialDownload, max_parts: Option<usize>) -> Result<usize> {
    Ok(get_parts_with_usage(endpoint, remote, partial, max_parts).await?.0)
}

pub async fn download(endpoint: &Endpoint, remote: EndpointAddr) -> Result<Vec<u8>> {
    let manifest = manifest_from(endpoint, remote.clone()).await?;
    let mut partial = PartialDownload::new(manifest)?;
    get_parts(endpoint, remote, &mut partial, None).await?;
    partial.finish()
}

/// For tests only: build an intentionally malformed content request.
#[cfg(test)]
fn request_part(digest: [u8; 32], index: u32) -> Vec<u8> {
    let mut req = vec![OP_CHUNK]; req.extend(digest); req.extend(index.to_be_bytes()); req
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(n: usize) -> Vec<u8> { (0..n).map(|i| (i as u8).wrapping_mul(37)).collect() }

    fn start(endpoint: Endpoint, book: Book, peer: EndpointId, global: u64, per_peer: u64)
        -> (tokio::task::JoinHandle<()>, Arc<Mutex<QuotaState>>) {
        let server = Server::new(book, HashSet::from([peer]), QuotaState::new(global, per_peer));
        let counter = server.quota();
        (tokio::spawn(server.run(endpoint)), counter)
    }

    #[test]
    fn checks_metadata_bounds() {
        let good = Book::new(bytes(700_017)).unwrap();
        let encoded = good.manifest.encode();
        assert_eq!(Manifest::decode(&encoded).unwrap(), good.manifest);
        assert!(Manifest::decode(&encoded[..encoded.len()-1]).is_err());
        let mut forged = encoded.clone(); forged[12..16].copy_from_slice(&0u32.to_be_bytes());
        assert!(Manifest::decode(&forged).is_err());
        let mut forged = encoded.clone(); forged[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Manifest::decode(&forged).is_err());
        assert!(Book::new(bytes(MAX_BOOK_BYTES+1)).is_err());
    }
    #[test]
    fn no_quota_writes_on_bad_requests() {
        let book = Book::new(bytes(345_000)).unwrap();
        let peer = SecretKey::generate().public();
        let service = Server::new(book.clone(), HashSet::from([peer]), QuotaState::new(1_000_000,1_000_000));
        assert_eq!(service.reply(peer,&[OP_CHUNK]), [BAD_REQUEST]);
        assert_eq!(service.reply(peer,&request_part([4u8;32],0)), [NOT_FOUND]);
        assert_eq!(service.reply(peer,&request_part(book.manifest.digest,999)), [NOT_FOUND]);
        assert_eq!(service.quota.lock().unwrap().used(),0);
    }
    #[test]
    fn integrity_rejects_corruption() {
        let book = Book::new(bytes(345_000)).unwrap();
        let mut partial=PartialDownload::new(book.manifest.clone()).unwrap();
        let mut first=book.part(0).unwrap().to_vec(); first[200] ^= 0xff;
        assert!(partial.add_part(0,first).is_err());
        assert_eq!(partial.verified_count(), 0);
        assert!(partial.finish().is_err());
    }
    #[test]
    fn quota_is_atomic_across_peers() {
        let a=SecretKey::generate().public(); let b=SecretKey::generate().public();
        let mut q=QuotaState::new(10,9);
        assert!(q.reserve(a,5)); assert!(q.reserve(b,5));
        assert!(!q.reserve(a,1)); assert!(!q.reserve(b,1));
        assert_eq!(q.used(),10); assert_eq!(q.used_for(&a),5);
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn real_noq_udp_connection_counters() -> Result<()> {
        let server_ep = private_endpoint(None).await?;
        let client_ep = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let (task, _) = start(server_ep.clone(), Book::new(vec![42; 1000])?,
                              client_ep.id(), 1_000_000, 1_000_000);
        let conn = client_ep.connect(addr, ALPN).await?;
        let reply = ask(&conn, &[OP_MANIFEST], MAX_MANIFEST_BYTES).await?;
        let _manifest = Manifest::decode(&reply)?;
        let usage = WireUsage::sample(&conn);
        assert!(usage.udp_tx_bytes > 0, "QUIC request generates outgoing UDP");
        assert!(usage.udp_rx_bytes > 0, "QUIC response generates incoming UDP");
        assert!(usage.udp_tx_datagrams > 0 && usage.udp_rx_datagrams > 0);
        assert_eq!(usage.connections, 1);
        let mut sum = WireUsage::default();
        sum.add(usage);
        sum.add(usage);
        assert_eq!(sum.udp_tx_bytes, usage.udp_tx_bytes * 2);
        conn.close(0u32.into(), b"done");
        client_ep.close().await;
        server_ep.close().await;
        task.abort();
        Ok(())
    }
    #[tokio::test(flavor="multi_thread",worker_threads=4)]
    async fn two_parallel_readers_share_global_quota_and_rate() -> Result<()> {
        let server_ep = private_endpoint(None).await?;
        let reader_a = private_endpoint(None).await?;
        let reader_b = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let book = Book::new(bytes(CHUNK_SIZE))?;
        let limits = QuotaState::new((CHUNK_SIZE * 2) as u64, CHUNK_SIZE as u64);
        let server = Server::new(book.clone(),
                HashSet::from([reader_a.id(), reader_b.id()]), limits)
            .with_rate_limit(rate_limit::RateGate::new(1024 * 1024, 512 * 1024)?);
        let quota = server.quota();
        let task = tokio::spawn(server.run(server_ep.clone()));
        let (a, b) = tokio::join!(
            download(&reader_a, addr.clone()),
            download(&reader_b, addr.clone())
        );
        assert_eq!(a?, book.bytes.as_ref().clone());
        assert_eq!(b?, book.bytes.as_ref().clone());
        assert_eq!(quota.lock().unwrap().used(), (CHUNK_SIZE * 2) as u64);
        assert_eq!(quota.lock().unwrap().used_for(&reader_a.id()), CHUNK_SIZE as u64);
        assert_eq!(quota.lock().unwrap().used_for(&reader_b.id()), CHUNK_SIZE as u64);
        assert!(download(&reader_a, addr.clone()).await.is_err(),
                "repeat download cannot exceed per-peer or global quota");
        reader_a.close().await; reader_b.close().await;
        server_ep.close().await; task.abort();
        Ok(())
    }
    #[tokio::test(flavor="multi_thread",worker_threads=8)]
    async fn ten_simultaneous_quic_readers_load_and_measure() -> Result<()> {
        use tokio::time::Instant;
        const N: usize = 10;
        let server_ep = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let mut endpoints = Vec::with_capacity(N);
        for _ in 0..N { endpoints.push(private_endpoint(None).await?); }
        let allowed: HashSet<_> = endpoints.iter().map(|ep| ep.id()).collect();
        let book = Book::new(bytes(CHUNK_SIZE))?;
        let global_bytes = (N * CHUNK_SIZE) as u64;
        let server = Server::new(book.clone(), allowed, QuotaState::new(global_bytes, CHUNK_SIZE as u64))
            .with_rate_limit(rate_limit::RateGate::new(2 * 1024 * 1024, 1024 * 1024)?);
        let quota = server.quota();
        let task = tokio::spawn(server.run(server_ep.clone()));
        let barrier = Arc::new(tokio::sync::Barrier::new(N+1));
        let mut jobs = Vec::new();
        for ep in &endpoints {
            let ep = ep.clone();
            let expected = book.manifest.digest;
            let addr = addr.clone();
            let gate = barrier.clone();
            jobs.push(tokio::spawn(async move {
                gate.wait().await;
                let start = Instant::now();
                let manifest = manifest_from(&ep, addr.clone()).await?;
                ensure!(manifest.digest == expected, "book mismatch");
                let mut partial = PartialDownload::new(manifest)?;
                let (count, usage) = get_parts_with_usage(&ep, addr, &mut partial, None).await?;
                let complete = partial.finish()?;
                ensure!(count == 1 && sha(&complete) == expected, "verified payload mismatch");
                Ok::<_, anyhow::Error>((start.elapsed(), usage.udp_rx_bytes, complete.len()))
            }));
        }
        barrier.wait().await;
        let started = Instant::now();
        let mut times = Vec::new();
        let mut udp_rx = 0u64;
        for job in jobs {
            let (elapsed, udp, size) = timeout(Duration::from_secs(40), job).await???;
            ensure!(size == CHUNK_SIZE, "unexpected payload size");
            times.push(elapsed);
            udp_rx = udp_rx.saturating_add(udp);
        }
        times.sort();
        let elapsed = started.elapsed();
        let p50 = times[4].as_millis();
        let p95 = times[9].as_millis();
        let slowest_ratio = times[9].as_secs_f64() / times[0].as_secs_f64().max(0.001);
        let accounted = quota.lock().unwrap();
        ensure!(accounted.used() == global_bytes, "global budget mismatch");
        for ep in &endpoints {
            ensure!(accounted.used_for(&ep.id()) == CHUNK_SIZE as u64, "peer quota mismatch");
        }
        println!("LOAD_10 clients=10 payload_bytes={global_bytes} udp_rx_bytes={udp_rx} elapsed_ms={} p50_ms={p50} p95_ms={p95} slowest_fastest_ratio={slowest_ratio:.3} max_connections={MAX_ACTIVE_CONNECTIONS}",
            elapsed.as_millis());
        ensure!(p95 < 40_000, "ten-client completion too slow");
        ensure!(slowest_ratio < 20.0, "slowest reader starved");
        drop(accounted);
        for ep in endpoints { ep.close().await; }
        server_ep.close().await;
        task.abort();
        Ok(())
    }
    /// Heavier transport test: real 25 QUIC endpoints, two 256-KiB chunks each,
    /// shared rate gate and quotas. Still one Linux host (NOT 25 devices/WAN).
    #[tokio::test(flavor="multi_thread",worker_threads=8)]
    async fn twenty_five_quic_readers_two_chunks_load_and_measure() -> Result<()> {
        use tokio::time::Instant;
        const N: usize = 25;
        const BOOK_BYTES: usize = 2 * CHUNK_SIZE;
        let server_ep = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let mut endpoints = Vec::with_capacity(N);
        for _ in 0..N { endpoints.push(private_endpoint(None).await?); }
        let allowed: HashSet<_> = endpoints.iter().map(|ep| ep.id()).collect();
        let book = Book::new(bytes(BOOK_BYTES))?;
        let global_bytes = (N * BOOK_BYTES) as u64;
        let server = Server::new(book.clone(), allowed, QuotaState::new(global_bytes, BOOK_BYTES as u64))
            .with_rate_limit(rate_limit::RateGate::new(2 * 1024 * 1024, 1024 * 1024)?);
        let quota = server.quota();
        let task = tokio::spawn(server.run(server_ep.clone()));
        let barrier = Arc::new(tokio::sync::Barrier::new(N + 1));
        let mut jobs = Vec::with_capacity(N);
        for ep in &endpoints {
            let ep = ep.clone();
            let expected = book.manifest.digest;
            let addr = addr.clone();
            let gate = barrier.clone();
            jobs.push(tokio::spawn(async move {
                gate.wait().await;
                let start = Instant::now();
                let manifest = manifest_from(&ep, addr.clone()).await?;
                ensure!(manifest.digest == expected, "book mismatch");
                let mut partial = PartialDownload::new(manifest)?;
                let (count, usage) = get_parts_with_usage(&ep, addr, &mut partial, None).await?;
                let complete = partial.finish()?;
                ensure!(count == 2 && complete.len() == BOOK_BYTES && sha(&complete) == expected,
                    "verified content/chunk mismatch");
                Ok::<_, anyhow::Error>((start.elapsed(), usage.udp_rx_bytes))
            }));
        }
        barrier.wait().await;
        let started = Instant::now();
        let mut times = Vec::with_capacity(N);
        let mut udp_rx = 0u64;
        for job in jobs {
            let (elapsed, udp) = timeout(Duration::from_secs(50), job).await???;
            times.push(elapsed);
            udp_rx = udp_rx.saturating_add(udp);
        }
        times.sort();
        let elapsed = started.elapsed();
        let p50 = times[12].as_millis(); // nearest rank ceil(.50 * 25) = 13
        let p95 = times[23].as_millis(); // nearest rank ceil(.95 * 25) = 24
        let max_ms = times[24].as_millis();
        let slowest_ratio = times[24].as_secs_f64() / times[0].as_secs_f64().max(0.001);
        let accounted = quota.lock().unwrap();
        ensure!(accounted.used() == global_bytes, "global budget mismatch");
        for ep in &endpoints {
            ensure!(accounted.used_for(&ep.id()) == BOOK_BYTES as u64, "peer quota mismatch");
        }
        let observed_over_payload = (udp_rx as f64) / (global_bytes as f64);
        println!("LOAD_25 clients=25 payload_bytes={global_bytes} udp_rx_bytes={udp_rx} rx_payload_ratio={observed_over_payload:.5} elapsed_ms={} p50_ms={p50} p95_ms={p95} max_ms={max_ms} slowest_fastest_ratio={slowest_ratio:.3} max_connections={MAX_ACTIVE_CONNECTIONS}",
            elapsed.as_millis());
        // Safety checks, not a real-world latency or fairness SLO.
        ensure!(p95 < 50_000, "25-client completion exceeded test timeout");
        ensure!(slowest_ratio < 20.0, "25-client reader appears starved");
        drop(accounted);
        for ep in endpoints { ep.close().await; }
        server_ep.close().await;
        task.abort();
        Ok(())
    }
    /// 50 clients race for at most 32 active server slots. The overload
    /// scenario exercises admission/backpressure as well as the per-peer
    /// and global byte/rate budgets. One Linux host, not 50 physical devices.
    #[tokio::test(flavor="multi_thread",worker_threads=8)]
    async fn fifty_quic_clients_backpressure_and_measure() -> Result<()> {
        use tokio::time::Instant;
        const N: usize = 50;
        const SIZE: usize = CHUNK_SIZE;
        ensure!(N > MAX_ACTIVE_CONNECTIONS, "test must exceed slot capacity");
        let server_ep = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let mut endpoints = Vec::with_capacity(N);
        for _ in 0..N { endpoints.push(private_endpoint(None).await?); }
        let allowed: HashSet<_> = endpoints.iter().map(|ep| ep.id()).collect();
        let book = Book::new(bytes(SIZE))?;
        let expected = book.manifest.digest;
        let budget = (N * SIZE) as u64;
        let server = Server::new(book, allowed, QuotaState::new(budget, SIZE as u64))
            .with_rate_limit(rate_limit::RateGate::new(8 * 1024 * 1024, 2 * 1024 * 1024)?);
        let quota = server.quota();
        let server_task = tokio::spawn(server.run(server_ep.clone()));
        let barrier = Arc::new(tokio::sync::Barrier::new(N + 1));
        let mut jobs = Vec::with_capacity(N);
        for ep in &endpoints {
            let ep = ep.clone();
            let addr = addr.clone();
            let gate = barrier.clone();
            jobs.push(tokio::spawn(async move {
                gate.wait().await;
                let start = Instant::now();
                let manifest = manifest_from(&ep, addr.clone()).await?;
                ensure!(manifest.digest == expected, "manifest digest mismatch");
                let mut partial = PartialDownload::new(manifest)?;
                let (count, usage) = get_parts_with_usage(&ep, addr, &mut partial, None).await?;
                let file = partial.finish()?;
                ensure!(count == 1 && file.len() == SIZE && sha(&file) == expected,
                    "book content mismatch");
                Ok::<_, anyhow::Error>((start.elapsed(), usage.udp_rx_bytes))
            }));
        }
        barrier.wait().await;
        let started = Instant::now();
        let mut times = Vec::with_capacity(N);
        let mut udp_rx = 0u64;
        for job in jobs {
            let (elapsed, udp) = timeout(Duration::from_secs(60), job).await???;
            times.push(elapsed);
            udp_rx = udp_rx.saturating_add(udp);
        }
        times.sort();
        let p50 = times[24].as_millis(); // nearest rank ceil(0.5*50)
        let p95 = times[47].as_millis(); // nearest rank ceil(0.95*50)
        let max_ms = times[49].as_millis();
        let slowest_ratio = times[49].as_secs_f64() / times[0].as_secs_f64().max(0.001);
        let used = quota.lock().unwrap();
        ensure!(used.used() == budget, "50-client total payload budget mismatch");
        for ep in &endpoints {
            ensure!(used.used_for(&ep.id()) == SIZE as u64, "per-peer budget mismatch");
        }
        println!("LOAD_50 clients=50 payload_bytes={budget} udp_rx_bytes={udp_rx} elapsed_ms={} p50_ms={p50} p95_ms={p95} max_ms={max_ms} slowest_fastest_ratio={slowest_ratio:.3} max_connections={MAX_ACTIVE_CONNECTIONS}",
            started.elapsed().as_millis());
        ensure!(p95 < 60_000, "50-client p95 exceeded transport timeout");
        // At most 32 concurrently accepted streams means a waiting wave can
        // legitimately be slower; this bound catches extreme starvation only.
        ensure!(slowest_ratio < 25.0, "some of 50 clients may have starved");
        drop(used);
        for ep in endpoints { ep.close().await; }
        server_ep.close().await;
        server_task.abort();
        Ok(())
    }
    #[tokio::test(flavor="multi_thread",worker_threads=4)]
    async fn idle_authorized_connections_release_slots() -> Result<()> {
        let server_ep = private_endpoint(None).await?;
        let attacker_ep = private_endpoint(None).await?;
        let healthy_ep = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let book = Book::new(bytes(2048))?;
        let allowed = HashSet::from([attacker_ep.id(), healthy_ep.id()]);
        let service = Server::new(book.clone(), allowed, QuotaState::new(100_000, 100_000))
            .with_request_timeouts(Duration::from_millis(250), Duration::from_millis(250));
        let active = service.active.clone();
        let worker = tokio::spawn(service.clone().run(server_ep.clone()));
        let first = attacker_ep.connect(addr.clone(), ALPN).await?;
        let second = attacker_ep.connect(addr.clone(), ALPN).await?;
        // The first two authorized connections hold slots but send no streams.
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(active.lock().unwrap().per_identity.get(&attacker_ep.id()).copied(), Some(2));
        let third = attacker_ep.connect(addr.clone(), ALPN).await?;
        let _ = timeout(Duration::from_secs(2), third.closed()).await?;
        let result = download(&healthy_ep, addr.clone()).await?;
        ensure!(result == *book.bytes, "healthy client blocked by idle peer");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(active.lock().unwrap().per_identity.get(&attacker_ep.id()).is_none(),
            "idle leases must expire and release slots");
        first.close(0u32.into(), b"done");
        second.close(0u32.into(), b"done");
        third.close(0u32.into(), b"done");
        attacker_ep.close().await;
        healthy_ep.close().await;
        server_ep.close().await;
        worker.abort();
        Ok(())
    }

    #[tokio::test(flavor="multi_thread",worker_threads=4)]
    async fn stalled_partial_request_is_evicted() -> Result<()> {
        let server_ep = private_endpoint(None).await?;
        let peer_ep = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let service = Server::new(Book::new(bytes(2048))?,
            HashSet::from([peer_ep.id()]), QuotaState::new(100_000,100_000))
            .with_request_timeouts(Duration::from_millis(500), Duration::from_millis(150));
        let active = service.active.clone();
        let worker = tokio::spawn(service.clone().run(server_ep.clone()));
        let conn = peer_ep.connect(addr, ALPN).await?;
        let (mut send, _recv) = conn.open_bi().await?;
        send.write_all(&[OP_CHUNK]).await?; // Never sends the rest of request.
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(active.lock().unwrap().per_identity.get(&peer_ep.id()).is_none(),
            "incomplete request must expire without reserving quota");
        assert_eq!(service.quota.lock().unwrap().used(),0);
        conn.close(0u32.into(), b"done");
        peer_ep.close().await;
        server_ep.close().await;
        worker.abort();
        Ok(())
    }
    #[test]
    fn many_rotated_keys_share_principal_admission_and_payload_budget() -> Result<()> {
        let keys: Vec<_> = (0..12).map(|_| SecretKey::generate().public()).collect();
        let principal = keys[0];
        let allowed: HashSet<_> = keys.iter().copied().collect();
        let mapping: HashMap<_,_> = keys.iter().map(|id| (*id, principal)).collect::<Vec<_>>()
            .into_iter().chain([(principal, principal)]).collect();
        let book=Book::new(bytes(CHUNK_SIZE))?;
        let server=Server::new(book.clone(), allowed, QuotaState::new((CHUNK_SIZE*2) as u64, CHUNK_SIZE as u64))
            .with_trusted_principals(mapping)?;
        let mut leases=Vec::new();
        for key in &keys[..8] { leases.push(server.admit_peer(*key).expect("first 8 admitted")); }
        for key in &keys[8..] { assert!(server.admit_peer(*key).is_none(), "rotated key bypassed principal cap"); }
        drop(leases);
        assert!(server.admit_peer(keys[11]).is_some(), "freed group capacity not reusable");
        let req=request_part(book.manifest.digest,0);
        assert_eq!(server.reply(principal,&req)[0],OK);
        assert_eq!(server.reply(principal,&req)[0],QUOTA);
        assert_eq!(server.quota.lock().unwrap().used(),CHUNK_SIZE as u64);
        Ok(())
    }
    #[test]
    fn untrusted_principal_configuration_fails_closed() -> Result<()> {
        let a=SecretKey::generate().public();
        let b=SecretKey::generate().public();
        let book=Book::new(vec![1,2,3])?;
        let mk=||Server::new(book.clone(),HashSet::from([a,b]),QuotaState::new(100,100));
        assert!(mk().with_trusted_principals(HashMap::from([(a,a)])).is_err());
        assert!(mk().with_trusted_principals(HashMap::from([(a,b),(b,a)])).is_err());
        let unknown=SecretKey::generate().public();
        assert!(mk().with_trusted_principals(HashMap::from([(a,unknown),(b,b)])).is_err());
        Ok(())
    }
    /// Linux-only process-level observations. This is CPU time in clock ticks and
    /// resident memory in KiB, not CPU percentage nor interface-level traffic.
    #[cfg(target_os = "linux")]
    fn process_load_snapshot() -> Option<(u64, u64, u64)> {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let suffix = stat.rsplit_once(')')?.1;
        let columns: Vec<_> = suffix.split_whitespace().collect();
        // First suffix item is state (field 3), so utime/stime (14/15) are 11/12.
        let cpu = columns.get(11)?.parse::<u64>().ok()?
            .saturating_add(columns.get(12)?.parse::<u64>().ok()?);
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let kib = |field: &str| -> Option<u64> {
            status.lines().find(|line| line.starts_with(field))?
                .split_whitespace().nth(1)?.parse().ok()
        };
        Some((cpu, kib("VmRSS:")?, kib("VmHWM:")?))
    }

    #[tokio::test(flavor="multi_thread",worker_threads=8)]
    async fn unknown_48_quic_peers_cannot_read_book_and_known_peer_recovers() -> Result<()> {
        const STRANGERS: usize = 48; // exceeds 32 handshake slots; localhost only
        let server_ep = private_endpoint(None).await?;
        let reader_ep = private_endpoint(None).await?;
        let addr = loopback_addr(&server_ep)?;
        let book = Book::new(bytes(CHUNK_SIZE))?;
        let quota_bytes = CHUNK_SIZE as u64;
        let service = Server::new(book.clone(), HashSet::from([reader_ep.id()]),
            QuotaState::new(quota_bytes, quota_bytes))
            .with_rate_limit(rate_limit::RateGate::new(4*1024*1024, 2*1024*1024)?);
        let quota = service.quota();
        let server_task = tokio::spawn(service.run(server_ep.clone()));
        #[cfg(target_os = "linux")]
        let before = process_load_snapshot();
        let start = tokio::time::Instant::now();

        let mut strangers = Vec::with_capacity(STRANGERS);
        for _ in 0..STRANGERS {
            strangers.push(private_endpoint(None).await?);
        }
        let barrier = Arc::new(tokio::sync::Barrier::new(STRANGERS+1));
        let mut jobs = Vec::with_capacity(STRANGERS);
        for ep in &strangers {
            let ep = ep.clone();
            let remote = addr.clone();
            let begin = barrier.clone();
            jobs.push(tokio::spawn(async move {
                begin.wait().await;
                let started = tokio::time::Instant::now();
                let result = timeout(Duration::from_secs(14), manifest_from(&ep, remote)).await;
                // Unauthorized book publication is a security failure. A timeout is
                // a rejection for this bounded lab test, not proof of a permanent ban.
                (matches!(&result, Ok(Ok(_))), result.is_err(), started.elapsed())
            }));
        }
        barrier.wait().await;
        // Try the authorized client while unknown endpoints are connecting.
        tokio::time::sleep(Duration::from_millis(35)).await;
        let mut known_file = None;
        let mut known_attempts = 0usize;
        for attempt in 0..5 {
            known_attempts = attempt + 1;
            match timeout(Duration::from_secs(6), download(&reader_ep, addr.clone())).await {
                Ok(Ok(bytes)) => { known_file = Some(bytes); break; }
                _ => tokio::time::sleep(Duration::from_millis(180)).await,
            }
        }

        let mut denied = 0usize;
        let mut timed_out = 0usize;
        let mut times = Vec::with_capacity(STRANGERS);
        for job in jobs {
            let (leaked, expired, elapsed) = timeout(Duration::from_secs(18), job).await??;
            ensure!(!leaked, "unknown QUIC key obtained a private book manifest");
            denied += 1;
            timed_out += usize::from(expired);
            times.push(elapsed);
        }
        ensure!(denied == STRANGERS, "unaccounted unknown peers");
        let known_file = known_file.context("authorized reader was starved by unknown peers")?;
        ensure!(sha(&known_file) == book.manifest.digest, "authorized download corrupted");
        ensure!(quota.lock().unwrap().used() == quota_bytes,
            "unknown attempts consumed protected book upload budget");
        times.sort();
        let p95_ms = times[45].as_millis(); // 46th of 48 = nearest-rank p95
        #[cfg(target_os = "linux")]
        {
            if let (Some((old_ticks, old_rss, _)), Some((now_ticks, now_rss, hwm))) =
                (before, process_load_snapshot()) {
                println!("UNKNOWN_48 denied={denied} timeouts={timed_out} known_attempts={known_attempts} elapsed_ms={} p95_denial_ms={p95_ms} cpu_ticks={} rss_delta_kib={} rss_now_kib={now_rss} rss_hwm_kib={hwm}",
                    start.elapsed().as_millis(), now_ticks.saturating_sub(old_ticks),
                    now_rss.saturating_sub(old_rss));
            }
        }
        #[cfg(not(target_os = "linux"))]
        println!("UNKNOWN_48 denied={denied} timeouts={timed_out} known_attempts={known_attempts} elapsed_ms={} p95_denial_ms={p95_ms}",
            start.elapsed().as_millis());
        for ep in strangers { ep.close().await; }
        reader_ep.close().await;
        server_ep.close().await;
        server_task.abort();
        Ok(())
    }

    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn real_iroh_direct_loopback_and_resume() -> Result<()> {
        let server_ep = private_endpoint(None).await?;
        let client_ep = private_endpoint(None).await?;
        let addr=loopback_addr(&server_ep)?;
        let data=bytes(CHUNK_SIZE*2+45);
        let book=Book::new(data.clone())?;
        let (task, quota)=start(server_ep.clone(),book,client_ep.id(),1_000_000,1_000_000);
        let mut partial=PartialDownload::new(manifest_from(&client_ep,addr.clone()).await?)?;
        assert_eq!(get_parts(&client_ep,addr.clone(), &mut partial, Some(1)).await?,1);
        assert_eq!(partial.verified_count(),1);
        assert_eq!(get_parts(&client_ep,addr, &mut partial,None).await?,2);
        assert_eq!(partial.finish()?,data);
        assert_eq!(quota.lock().unwrap().used(),data.len() as u64);
        client_ep.close().await;
        server_ep.close().await;
        task.abort();
        Ok(())
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn real_iroh_rejects_unknown_peer() -> Result<()> {
        let server_ep=private_endpoint(None).await?;
        let allowed_ep=private_endpoint(None).await?;
        let attacker=private_endpoint(None).await?;
        let addr=loopback_addr(&server_ep)?;
        let book=Book::new(bytes(CHUNK_SIZE+4))?;
        let (task,quota)=start(server_ep.clone(),book,allowed_ep.id(),1000000,1000000);
        assert!(manifest_from(&attacker,addr).await.is_err());
        assert_eq!(quota.lock().unwrap().used(),0);
        attacker.close().await; allowed_ep.close().await; server_ep.close().await; task.abort();
        Ok(())
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn real_iroh_enforces_payload_quota() -> Result<()> {
        let server_ep=private_endpoint(None).await?;
        let client_ep=private_endpoint(None).await?;
        let addr=loopback_addr(&server_ep)?;
        let book=Book::new(bytes(CHUNK_SIZE+42))?;
        let (task,quota)=start(server_ep.clone(),book,client_ep.id(),CHUNK_SIZE as u64,CHUNK_SIZE as u64);
        let mut partial=PartialDownload::new(manifest_from(&client_ep,addr.clone()).await?)?;
        assert!(get_parts(&client_ep,addr,&mut partial,None).await.is_err());
        assert_eq!(partial.verified_count(),1);
        assert_eq!(quota.lock().unwrap().used(),CHUNK_SIZE as u64);
        client_ep.close().await;server_ep.close().await;task.abort();
        Ok(())
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn rebind_preserves_identity_and_download() -> Result<()> {
        let secret=SecretKey::generate();
        let client_ep=private_endpoint(None).await?;
        let old_ep=private_endpoint(Some(secret.clone())).await?;
        let old_addr=loopback_addr(&old_ep)?;
        old_ep.close().await;
        let new_ep=private_endpoint(Some(secret)).await?;
        let new_addr=loopback_addr(&new_ep)?;
        assert_eq!(old_addr.id,new_addr.id);
        let book=Book::new(bytes(CHUNK_SIZE+39))?;
        let (task,_quota)=start(new_ep.clone(),book.clone(),client_ep.id(),10_000_000,10_000_000);
        assert_eq!(download(&client_ep,new_addr).await?,book.bytes.as_ref().clone());
        client_ep.close().await;new_ep.close().await;task.abort();
        Ok(())
    }
}