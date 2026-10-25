//! Actual UDP impairment between two Iroh/QUIC endpoints. Unlike pacing the
//! application stream, this dedicated loopback proxy discards real datagrams,
//! delays others, injects reordering opportunities, and rate-limits forwarding.
//! Two distinct UDP forwarding routes simulate a route replacement; neither is
//! an actual Wi-Fi/LTE interface, cellular handover, NAT or two-host field test.
//! The existing shipping beta wire protocol and its permissions are untouched.
use super::*;
use super::resume_checkpoint_lab::{Checkpoint, Identity};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const PHOTO_SIZE: usize = 5 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const REQUEST_MAGIC: &[u8; 8] = b"AKLOSS01";
const PARTIAL: &[u8] = b"CHECKPOINT_ONLY_NOT_DELIVERED";
const COMPLETE: &[u8] = b"LOCAL_INBOX_COMPLETE_NOT_W4";
const READ_ACK: &[u8] = b"RECEIPT_CONSUMED";

#[derive(Clone, Copy)]
struct Profile {
    latency: Duration,
    extra_reorder_delay: Duration,
    bytes_per_second: u64,
    loss_every: usize,
    reordered_every: usize,
}

#[derive(Default)]
struct Counters {
    inbound: AtomicUsize,
    inbound_bytes: AtomicUsize,
    forwarded: AtomicUsize,
    forwarded_bytes: AtomicUsize,
    dropped: AtomicUsize,
    reordered: AtomicUsize,
    queue_peak: AtomicUsize,
}

struct Pending {
    due: Instant,
    data: Vec<u8>,
    to_server: bool,
    destination: SocketAddr,
}

fn enqueue(
    buffer: &[u8],
    to_server: bool,
    destination: SocketAddr,
    profile: Profile,
    count: &mut usize,
    next_tx: &mut Instant,
    pending: &mut Vec<Pending>,
    stats: &Counters,
) {
    *count += 1;
    stats.inbound.fetch_add(1, Ordering::Relaxed);
    stats.inbound_bytes.fetch_add(buffer.len(), Ordering::Relaxed);
    if *count % profile.loss_every == 0 || pending.len() >= 4096 {
        stats.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let now = Instant::now();
    let begin = if *next_tx > now { *next_tx } else { now };
    let transmission = Duration::from_secs_f64(
        buffer.len() as f64 / profile.bytes_per_second as f64,
    );
    *next_tx = begin + transmission;
    let mut due = *next_tx + profile.latency;
    // The next datagram is allowed to overtake this one when the profile
    // adds enough delay. These are packet-level, not app-chunk delays.
    if *count % profile.reordered_every == 0 {
        due += profile.extra_reorder_delay;
        stats.reordered.fetch_add(1, Ordering::Relaxed);
    }
    pending.push(Pending {
        due,
        data: buffer.to_vec(),
        to_server,
        destination,
    });
    stats.queue_peak.fetch_max(pending.len(), Ordering::Relaxed);
}

struct ImpairedPath {
    listen: SocketAddr,
    stop: Arc<AtomicBool>,
    stats: Arc<Counters>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ImpairedPath {
    fn start(server: SocketAddr, profile: Profile) -> Self {
        let facing_client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let facing_server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        facing_client.set_nonblocking(true).unwrap();
        facing_server.set_nonblocking(true).unwrap();
        let listen = facing_client.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Counters::default());
        let stop_worker = Arc::clone(&stop);
        let stats_worker = Arc::clone(&stats);
        let worker = thread::spawn(move || {
            let mut front_buf = [0u8; 65536];
            let mut back_buf = [0u8; 65536];
            let mut client: Option<SocketAddr> = None;
            let mut seq_to_server = 0usize;
            let mut seq_to_client = 0usize;
            let mut next_server = Instant::now();
            let mut next_client = Instant::now();
            let mut pending = Vec::<Pending>::new();
            while !stop_worker.load(Ordering::Relaxed) {
                loop {
                    match facing_client.recv_from(&mut front_buf) {
                        Ok((len, addr)) => {
                            if let Some(previous) = client {
                                if previous != addr {
                                    // Fail closed on an unrelated sender to
                                    // avoid an open-UDP-reflector laboratory.
                                    stats_worker.dropped.fetch_add(1, Ordering::Relaxed);
                                    continue;
                                }
                            } else {
                                client = Some(addr);
                            }
                            enqueue(
                                &front_buf[..len], true, server, profile,
                                &mut seq_to_server, &mut next_server,
                                &mut pending, &stats_worker,
                            );
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => panic!("loopback proxy frontend: {e}"),
                    }
                }
                loop {
                    match facing_server.recv_from(&mut back_buf) {
                        Ok((len, addr)) => {
                            if addr != server {
                                stats_worker.dropped.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            if let Some(peer) = client {
                                enqueue(
                                    &back_buf[..len], false, peer, profile,
                                    &mut seq_to_client, &mut next_client,
                                    &mut pending, &stats_worker,
                                );
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => panic!("loopback proxy backend: {e}"),
                    }
                }
                let now = Instant::now();
                let mut index = 0usize;
                while index < pending.len() {
                    if pending[index].due <= now {
                        let item = pending.swap_remove(index);
                        let socket = if item.to_server {
                            &facing_server
                        } else {
                            &facing_client
                        };
                        if socket.send_to(&item.data, item.destination).is_ok() {
                            stats_worker.forwarded.fetch_add(1, Ordering::Relaxed);
                            stats_worker.forwarded_bytes.fetch_add(item.data.len(), Ordering::Relaxed);
                        } else {
                            stats_worker.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        index += 1;
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            listen,
            stop,
            stats,
            worker: Some(worker),
        }
    }

    fn address(&self, id: iroh::EndpointId) -> iroh::EndpointAddr {
        EndpointAddr::new(id).with_ip_addr(self.listen)
    }

    fn assert_injected(&self, minimum_forwarded_bytes: usize) {
        let packets = self.stats.inbound.load(Ordering::Relaxed);
        let bytes = self.stats.inbound_bytes.load(Ordering::Relaxed);
        let forwarded = self.stats.forwarded.load(Ordering::Relaxed);
        let delivered_bytes = self.stats.forwarded_bytes.load(Ordering::Relaxed);
        let dropped = self.stats.dropped.load(Ordering::Relaxed);
        let reordered = self.stats.reordered.load(Ordering::Relaxed);
        eprintln!(
            "UDP_IMPAIRMENT packets={packets} bytes={bytes} forwarded={forwarded}              forwarded_bytes={delivered_bytes} dropped={dropped} reorder_delayed={reordered}"
        );
        assert!(
            delivered_bytes >= minimum_forwarded_bytes,
            "real QUIC payload did not traverse required UDP proxy; this is bypass, not a passed impairment test",
        );
        assert!(packets > 100, "UDP test sampled too few datagrams");
        assert!(
            self.stats.forwarded.load(Ordering::Relaxed) > 100,
            "the impaired proxy must forward useful packets",
        );
        assert!(
            self.stats.dropped.load(Ordering::Relaxed) > 0,
            "packet-loss injections must actually occur",
        );
        assert!(
            self.stats.reordered.load(Ordering::Relaxed) > 0,
            "reordering delays must actually be introduced",
        );
    }
}

impl Drop for ImpairedPath {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("UDP impairment proxy must exit");
        }
    }
}

fn test_photo() -> Vec<u8> {
    let mut seed = 0xcb82_faf3_3610_18d9u64;
    let mut data = vec![0u8; PHOTO_SIZE];
    for byte in &mut data {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *byte = (seed >> 24) as u8;
    }
    data
}

/// Socket-level validation of the deterministic UDP impairment proxy.
/// Unlike the Iroh case below, the sender uses only the proxy's address:
/// a direct path cannot be discovered. This proves loss/forwarding/reorder
/// scheduling in the proxy, NOT QUIC resilience under packet loss.
/// A separate Linux tc/netem gate exercises the actual Iroh/QUIC path.
#[test]
fn raw_udp_socket_loss_is_observed_and_never_claimed_as_quic_evidence() {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    server
        .set_read_timeout(Some(Duration::from_millis(250)))
        .unwrap();
    let proxy = ImpairedPath::start(
        server.local_addr().unwrap(),
        Profile {
            latency: Duration::from_millis(3),
            extra_reorder_delay: Duration::from_millis(35),
            bytes_per_second: 320_000,
            loss_every: 5,
            reordered_every: 7,
        },
    );
    let receiver = thread::spawn(move || {
        let mut count = 0usize;
        let mut last = None;
        let mut out_of_order = 0usize;
        let mut buffer = [0u8; 1024];
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match server.recv_from(&mut buffer) {
                Ok((len, _)) => {
                    assert_eq!(len, 512, "fixed datagrams only");
                    assert!(buffer[4..len].iter().all(|b| *b == 0xa5));
                    let seq = u32::from_be_bytes(buffer[..4].try_into().unwrap());
                    if last.is_some_and(|previous| seq < previous) {
                        out_of_order += 1;
                    }
                    last = Some(seq);
                    count += 1;
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if count > 100 { break; }
                }
                Err(e) => panic!("UDP receiver unexpectedly failed: {e}"),
            }
        }
        (count, out_of_order)
    });
    let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    for seq in 0..240u32 {
        let mut packet = [0xa5u8; 512];
        packet[..4].copy_from_slice(&seq.to_be_bytes());
        assert_eq!(client.send_to(&packet, proxy.listen).unwrap(), packet.len());
        thread::sleep(Duration::from_millis(2));
    }
    let (received, reordered_in_receiver) = receiver.join().unwrap();
    assert!(received > 100, "proxy did not carry enough real UDP datagrams");
    assert!(received < 240, "deterministic packet-loss profile was not enforced");
    proxy.assert_injected(80_000);
    eprintln!("RAW_UDP_PROXY received={received} reordered_receiver={reordered_in_receiver} QUIC_NOT_TESTED=true");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Iroh may discover a direct QUIC path around the proxy: diagnostic only. Use Linux tc/netem for QUIC loss."]
async fn actual_udp_packet_loss_throttle_jitter_route_switch_and_cold_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let inbox_root = dir.path().join("inbox");
    fs::create_dir(&inbox_root).unwrap();
    let partial_path = dir.path().join("checkpointed-photo.stage");
    let all = test_photo();
    let full_hash = *blake3::hash(&all).as_bytes();

    // Force a single IPv4 loopback socket and disable implicit discovery.
    // If iroh still learns another direct path the byte counters below
    // detect the bypass and this test MUST fail, not claim packet-loss proof.
    let sender = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0)).unwrap()
        .bind()
        .await
        .unwrap();
    let sender_id = sender.id();
    let server_port = sender
        .bound_sockets()
        .into_iter()
        .find(|addr| addr.is_ipv4())
        .expect("IPv4 Iroh socket for UDP proxy")
        .port();
    let server = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), server_port);
    let route_fast = ImpairedPath::start(
        server,
        Profile {
            latency: Duration::from_millis(8),
            extra_reorder_delay: Duration::from_millis(40),
            bytes_per_second: 1_500_000,
            loss_every: 47,
            reordered_every: 19,
        },
    );
    let route_slow = ImpairedPath::start(
        server,
        Profile {
            latency: Duration::from_millis(65),
            extra_reorder_delay: Duration::from_millis(90),
            bytes_per_second: 375_000,
            loss_every: 17,
            reordered_every: 13,
        },
    );

    let receiver = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0)).unwrap()
        .bind()
        .await
        .unwrap();
    let receiver_id = receiver.id();
    let identity = Identity {
        sender: *sender_id.as_bytes(),
        receiver: *receiver_id.as_bytes(),
        full_hash,
        total: PHOTO_SIZE as u64,
    };
    // A -> B -> A -> B. This changes actual UDP proxy sockets,
    // IP packet drop/delay/reorder profiles and available bandwidth.
    let sizes = [1024 * 1024usize, 1024 * 1024, 1024 * 1024, 2 * 1024 * 1024];
    assert_eq!(sizes.iter().sum::<usize>(), PHOTO_SIZE);
    let transmitted = all.clone();
    let send_job = tokio::spawn(async move {
        let mut offset = 0usize;
        for (index, bytes_to_send) in sizes.iter().copied().enumerate() {
            let incoming = tokio::time::timeout(Duration::from_secs(70), sender.accept())
                .await
                .unwrap()
                .expect("selected peer must reconnect through an impaired UDP route");
            let connection = incoming.accept().unwrap().await.unwrap();
            assert_eq!(connection.remote_id(), receiver_id);
            let mut request = connection.accept_uni().await.unwrap();
            let claim = request.read_to_end(64).await.unwrap();
            assert_eq!(claim.len(), 48);
            assert_eq!(&claim[..8], REQUEST_MAGIC);
            let start = u64::from_be_bytes(claim[8..16].try_into().unwrap());
            assert_eq!(start, offset as u64);
            assert_eq!(
                &claim[16..48],
                blake3::hash(&transmitted[..offset]).as_bytes(),
                "sender must verify actual prefix before avoiding retransmission",
            );
            let mut stream = connection.open_uni().await.unwrap();
            for part in transmitted[offset..offset + bytes_to_send].chunks(CHUNK) {
                stream.write_all(part).await.unwrap();
            }
            stream.finish().unwrap();
            let mut receipt = connection.accept_uni().await.unwrap();
            let status = receipt.read_to_end(64).await.unwrap();
            let expected = if index == sizes.len() - 1 {
                COMPLETE
            } else {
                PARTIAL
            };
            assert_eq!(status, expected);
            let mut delivered = connection.open_uni().await.unwrap();
            delivered.write_all(READ_ACK).await.unwrap();
            delivered.finish().unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            connection.close(0u8.into(), b"close tested proxy route");
            offset += bytes_to_send;
        }
        sender.close().await;
    });

    let mut confirmed = 0usize;
    for (index, bytes_to_receive) in sizes.iter().copied().enumerate() {
        let route = if index % 2 == 0 {
            &route_fast
        } else {
            &route_slow
        };
        let connection = tokio::time::timeout(
            Duration::from_secs(70), receiver.connect(route.address(sender_id), ALPN),
        )
        .await
        .unwrap()
        .expect("Iroh/QUIC should tolerate UDP losses via this relay");
        assert_eq!(connection.remote_id(), sender_id);
        let mut request = connection.open_uni().await.unwrap();
        let mut claim = Vec::with_capacity(48);
        claim.extend_from_slice(REQUEST_MAGIC);
        claim.extend_from_slice(&(confirmed as u64).to_be_bytes());
        let cold_prefix = if confirmed == 0 {
            Vec::new()
        } else {
            fs::read(&partial_path).unwrap()
        };
        assert_eq!(cold_prefix.len(), confirmed);
        claim.extend_from_slice(blake3::hash(&cold_prefix).as_bytes());
        request.write_all(&claim).await.unwrap();
        request.finish().unwrap();
        let mut stream = connection.accept_uni().await.unwrap();
        let bytes = tokio::time::timeout(
            Duration::from_secs(90), stream.read_to_end(bytes_to_receive),
        )
        .await
        .expect("deadline for rate-limited QUIC stream")
        .expect("QUIC loss recovery and reordering must preserve stream bytes");
        assert_eq!(bytes.len(), bytes_to_receive);
        assert_eq!(&bytes, &all[confirmed..confirmed + bytes_to_receive]);

        let mut stage = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&partial_path)
            .unwrap();
        stage.write_all(&bytes).unwrap();
        stage.sync_all().unwrap();
        drop(stage);
        confirmed += bytes.len();

        let physical = fs::read(&partial_path).unwrap();
        assert_eq!(physical.len(), confirmed);
        let prefix_hash = *blake3::hash(&physical).as_bytes();
        assert_eq!(prefix_hash, *blake3::hash(&all[..confirmed]).as_bytes());
        if confirmed != PHOTO_SIZE {
            // No partial delivery acknowledgment or permanent Inbox publish.
            let cp = Checkpoint {
                identity,
                offset: confirmed as u64,
                prefix_hash,
            };
            assert_eq!(Checkpoint::decode(&cp.wire()).unwrap(), cp);
            assert_eq!(fs::read_dir(&inbox_root).unwrap().count(), 0);
        } else {
            assert_eq!(prefix_hash, full_hash);
            let saved = inbox::persist_received_payload(
                &physical,
                &inbox_root,
                PHOTO_SIZE as u64,
                full_hash,
                *sender_id.as_bytes(),
                *receiver_id.as_bytes(),
            )
            .unwrap();
            let cold = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
            assert_eq!(cold.count().unwrap(), 1);
            assert_eq!(fs::read(cold.object_path(saved.id)).unwrap(), all);
        }
        let status = if index == sizes.len() - 1 {
            COMPLETE
        } else {
            PARTIAL
        };
        let mut reply = connection.open_uni().await.unwrap();
        reply.write_all(status).await.unwrap();
        reply.finish().unwrap();
        let mut consumed = connection.accept_uni().await.unwrap();
        assert_eq!(consumed.read_to_end(64).await.unwrap(), READ_ACK);
        connection.close(0u8.into(), b"switch to another UDP route");
    }
    receiver.close().await;
    tokio::time::timeout(Duration::from_secs(100), send_job)
        .await
        .unwrap()
        .unwrap();

    // The statistics prove actual UDP packet loss, not artificial sleeps in
    // the application stream. Both routes must have transmitted packets.
    // Sender payload (not total packet overhead): fast carries 2 MiB,
    // slow carries 3 MiB. Proxied UDP traffic must include almost all
    // application data rather than just ICE/path-discovery probes.
    route_fast.assert_injected(2 * 1024 * 1024);
    route_slow.assert_injected(3 * 1024 * 1024);
    assert_eq!(confirmed, PHOTO_SIZE);
}
