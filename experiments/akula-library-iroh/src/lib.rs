//! Isolated Akula Library research experiment. Not production, no WAN/relay discovery.
//! Iroh 1.3.0 QUIC authenticates peer endpoint IDs. Application access is an allowlist.
//! Explicit quotas below bound file payload bytes, NOT actual wire bytes including QUIC/IP.

use anyhow::{bail, ensure, Context, Result};
pub mod durable;
pub mod persistent_quota;
pub mod rate_limit;
use iroh::{endpoint::{presets, Connection}, Endpoint, EndpointAddr, EndpointId, SecretKey};
use std::{collections::{HashMap, HashSet}, net::SocketAddr, sync::{Arc, Mutex}};
use tokio::{sync::Semaphore, time::{timeout, Duration}};

pub const ALPN: &[u8] = b"akula-library/iroh-lab/1";
pub const CHUNK_SIZE: usize = 256 * 1024;
pub const MAX_BOOK_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHUNKS: usize = MAX_BOOK_BYTES / CHUNK_SIZE;
const MANIFEST_MAGIC: &[u8; 4] = b"AKM1";
const MAX_MANIFEST_BYTES: usize = 4 + 8 + 4 + 4 + 32 + MAX_CHUNKS * 32;
const REQUEST_LIMIT: usize = 1 + 32 + 4;
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

#[derive(Clone)]
pub struct Server {
    book: Book,
    allowed: Arc<HashSet<EndpointId>>,
    quota: Arc<Mutex<QuotaState>>,
    disk_quota: Option<Arc<Mutex<persistent_quota::PersistentQuota>>>,
    rate: Option<Arc<rate_limit::RateGate>>,
}
impl Server {
    pub fn new(book: Book, allowed: HashSet<EndpointId>, limits: QuotaState) -> Self {
        Self { book, allowed: Arc::new(allowed), quota: Arc::new(Mutex::new(limits)), disk_quota: None, rate: None }
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
    /// Run until the endpoint is closed; bounded to eight concurrent accepted connections.
    pub async fn run(self, endpoint: Endpoint) {
        let slots = Arc::new(Semaphore::new(8));
        loop {
            let Ok(permit) = slots.clone().acquire_owned().await else { break };
            let Some(incoming) = endpoint.accept().await else { break };
            let server = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                match incoming.await {
                    Ok(conn) => { let _ = server.handle_connection(conn).await; },
                    Err(_e) => (),
                }
            });
        }
    }
    async fn handle_connection(&self, conn: Connection) -> Result<()> {
        let peer = conn.remote_id();
        let result = self.handle_connection_inner(&conn, peer).await;
        let usage = WireUsage::sample(&conn);
        println!("SERVER_WIRE_METRICS peer={peer} udp_tx_bytes={} udp_rx_bytes={} udp_tx_datagrams={} udp_rx_datagrams={}",
            usage.udp_tx_bytes, usage.udp_rx_bytes, usage.udp_tx_datagrams, usage.udp_rx_datagrams);
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        result
    }
    async fn handle_connection_inner(&self, conn: &Connection, peer: EndpointId) -> Result<()> {
        if !self.allowed.contains(&peer) {
            conn.close(403u32.into(), b"denied");
            return Ok(());
        }
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            // A peer cannot force unlimited receive buffering via malformed messages.
            let incoming = timeout(RESPONSE_TIMEOUT, recv.read_to_end(REQUEST_LIMIT)).await;
            let response: Vec<u8> = match incoming {
                Ok(Ok(raw)) => self.reply(peer, &raw),
                _ => vec![BAD_REQUEST],
            };
            if let Some(gate) = &self.rate {
                // Pacing applies to all application response bytes including manifest/status.
                // The persistent ledger separately accounts only accepted file payload.
                for piece in response.chunks(rate_limit::PACE_SLICE) {
                    gate.wait(peer, piece.len()).await?;
                    timeout(RESPONSE_TIMEOUT, send.write_all(piece)).await??;
                }
            } else {
                timeout(RESPONSE_TIMEOUT, send.write_all(&response)).await??;
            }
            send.finish()?;
        }
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