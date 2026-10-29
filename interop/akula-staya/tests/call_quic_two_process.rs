#![cfg(feature = "call-quic-lab")]
//! AK-CALL-01: two *independent OS processes* exchange real, authenticated
//! Iroh/QUIC signaling over loopback. Both peers use throwaway keys.
//! The local test controller supplies synthetic, independently selected peers;
//! this is NOT OS pinning, durable replay defense or production call consent.
//! No microphone, speaker, camera, screen, file transfer or Vault is opened.

use aquilum_core::{
    call_signaling_lab::{CallPhase, CallSession, LocalPeerPolicy, RemoteEvent, WIRE_BYTES},
    p2p_offer_lab::PeerBinding,
    sync::DeviceId,
};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, endpoint::presets};
use std::{
    io::{BufRead, BufReader, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    process::{Child, Command, Stdio},
    str::FromStr,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const ALPN: &[u8] = b"akula/call-control-quic/lab/1";
const DEADLINE: Duration = Duration::from_secs(25);
const WAIT: Duration = Duration::from_secs(45);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs()
}

fn binding(device: u128, endpoint: EndpointId) -> PeerBinding {
    PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_u128(device)),
        endpoint_id: *endpoint.as_bytes(),
    }
}

async fn accept_peer(endpoint: &Endpoint, expected: EndpointId) -> iroh::endpoint::Connection {
    let incoming = tokio::time::timeout(DEADLINE, endpoint.accept())
        .await
        .expect("incoming Iroh timeout")
        .expect("Iroh endpoint closed unexpectedly");
    let handshake = incoming.accept().expect("real QUIC TLS handshake");
    let connection = tokio::time::timeout(DEADLINE, handshake)
        .await
        .expect("incoming handshake timeout")
        .expect("authenticated QUIC connection");
    assert_eq!(connection.remote_id(), expected, "wrong authenticated Iroh key");
    connection
}

async fn receive_offer(
    connection: &iroh::endpoint::Connection,
    endpoint: &Endpoint,
) -> (CallSession, iroh::endpoint::SendStream, Vec<u8>) {
    let (sender, mut receiver) = tokio::time::timeout(DEADLINE, connection.accept_bi())
        .await
        .expect("offer stream timeout")
        .expect("offer stream");
    let offer = tokio::time::timeout(DEADLINE, receiver.read_to_end(WIRE_BYTES))
        .await
        .expect("offer read timeout")
        .expect("bounded offer");
    assert_eq!(offer.len(), WIRE_BYTES);
    let session = CallSession::incoming(
        &offer,
        binding(1, connection.remote_id()),
        binding(2, endpoint.id()),
        LocalPeerPolicy::Allowed,
        now(),
    )
    .expect("independent authenticated QUIC identity matches synthetic test pin");
    (session, sender, offer)
}

async fn server(reconnect: bool) {
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("real Iroh listener");
    let socket = endpoint
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .or_else(|| endpoint.bound_sockets().into_iter().next())
        .expect("Iroh UDP listener socket");
    let loopback = match socket {
        SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    println!("READY {} {}", endpoint.id(), SocketAddr::new(loopback, socket.port()));
    std::io::stdout().flush().expect("flush READY");
    let mut command = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut command)
        .expect("read independently selected caller key");
    let parts: Vec<_> = command.split_whitespace().collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], "EXPECT");
    let expected = EndpointId::from_str(parts[1]).expect("selected peer ID");

    let mut old_call_id = None;
    if reconnect {
        let connection = accept_peer(&endpoint, expected).await;
        let (mut session, mut reply, offer) = receive_offer(&connection, &endpoint).await;
        old_call_id = Some(offer[11..27].to_vec());
        let ringing = session.ring_local(now()).expect("first ringing");
        reply.write_all(&ringing).await.expect("first ringing write");
        reply.finish().expect("finish first ringing stream");
        tokio::time::timeout(DEADLINE, reply.stopped())
            .await
            .expect("ringing delivery timeout")
            .expect("ringing stream completion");
        tokio::time::timeout(DEADLINE, connection.closed())
            .await
            .expect("test caller must close first QUIC connection");
        session.transport_lost();
        assert_eq!(session.phase(), CallPhase::Closed);
        assert!(session.end_local().expect("closed session").is_none());
        println!("FIRST_PATH_LOST_AND_CLOSED");
        connection.close(0u8.into(), b"closed by caller");
    }

    let connection = accept_peer(&endpoint, expected).await;
    let (mut session, mut reply, offer) = receive_offer(&connection, &endpoint).await;
    if let Some(old) = old_call_id {
        assert_ne!(&offer[11..27], old.as_slice(), "call ID must change after transport loss");
    }
    let ringing = session.ring_local(now()).expect("ringing");
    let acceptance = session.accept_local(now()).expect("accept with separate host consent");
    reply.write_all(&ringing).await.expect("send Ringing");
    reply.write_all(&acceptance).await.expect("send Accept");
    reply.finish().expect("finish response stream");

    let (mut ack, mut end_stream) = tokio::time::timeout(DEADLINE, connection.accept_bi())
        .await
        .expect("End stream timeout")
        .expect("End stream");
    let end = tokio::time::timeout(DEADLINE, end_stream.read_to_end(WIRE_BYTES))
        .await
        .expect("End packet timeout")
        .expect("bounded End packet");
    assert_eq!(
        session.receive(
            &end,
            binding(1, connection.remote_id()),
            LocalPeerPolicy::Allowed,
            now(),
        ),
        Ok(RemoteEvent::Ended)
    );
    assert_eq!(session.phase(), CallPhase::Closed);
    ack.write_all(b"AKULA_CALL_END_OK").await.expect("send acknowledgement");
    ack.finish().expect("finish acknowledgement");
    tokio::time::timeout(DEADLINE, ack.stopped())
        .await
        .expect("acknowledgement delivery timeout")
        .expect("acknowledgement stream completion");
    println!("CHILD_PASS server real_quic=1 reconnect={}", u8::from(reconnect));
    connection.close(0u8.into(), b"call complete");
    endpoint.close().await;
}

async fn connect_peer(
    endpoint: &Endpoint,
    expected: EndpointId,
    socket: SocketAddr,
) -> iroh::endpoint::Connection {
    let address = EndpointAddr::new(expected).with_ip_addr(socket);
    let connection = tokio::time::timeout(DEADLINE, endpoint.connect(address, ALPN))
        .await
        .expect("connect timeout")
        .expect("genuine Iroh/QUIC connect");
    assert_eq!(connection.remote_id(), expected, "wrong server TLS key");
    connection
}

async fn offer(
    endpoint: &Endpoint,
    connection: &iroh::endpoint::Connection,
    full_call: bool,
) {
    let (mut session, wire) = CallSession::outbound(
        binding(1, endpoint.id()),
        binding(2, connection.remote_id()),
        *Uuid::new_v4().as_bytes(),
        now(),
        60,
        LocalPeerPolicy::Allowed,
    )
    .expect("new test call with fresh random UUID");
    let (mut sender, mut receiver) = tokio::time::timeout(DEADLINE, connection.open_bi())
        .await
        .expect("offer stream open timeout")
        .expect("offer stream");
    sender.write_all(&wire).await.expect("send Offer");
    sender.finish().expect("finish Offer");
    let expected_bytes = if full_call { WIRE_BYTES * 2 } else { WIRE_BYTES };
    let replies = tokio::time::timeout(DEADLINE, receiver.read_to_end(expected_bytes))
        .await
        .expect("Ringing/Accept response timeout")
        .expect("bounded control response");
    assert_eq!(replies.len(), expected_bytes);
    assert_eq!(
        session.receive(
            &replies[..WIRE_BYTES],
            binding(2, connection.remote_id()),
            LocalPeerPolicy::Allowed,
            now(),
        ),
        Ok(RemoteEvent::Ringing)
    );
    if !full_call {
        assert_eq!(session.phase(), CallPhase::RemoteRinging);
        return;
    }
    assert_eq!(
        session.receive(
            &replies[WIRE_BYTES..],
            binding(2, connection.remote_id()),
            LocalPeerPolicy::Allowed,
            now(),
        ),
        Ok(RemoteEvent::Accepted)
    );
    assert_eq!(session.phase(), CallPhase::Active);
    let end = session.end_local().expect("user-initiated end").expect("End packet");
    let (mut sender, mut receiver) = tokio::time::timeout(DEADLINE, connection.open_bi())
        .await
        .expect("End stream open timeout")
        .expect("End stream");
    sender.write_all(&end).await.expect("send End");
    sender.finish().expect("finish End stream");
    let ack = tokio::time::timeout(DEADLINE, receiver.read_to_end(32))
        .await
        .expect("End acknowledgement timeout")
        .expect("bounded End acknowledgement");
    assert_eq!(ack, b"AKULA_CALL_END_OK");
    assert_eq!(session.phase(), CallPhase::Closed);
}

async fn client(reconnect: bool) {
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("real Iroh client endpoint");
    println!("READY {}", endpoint.id());
    std::io::stdout().flush().expect("flush READY");
    let mut command = String::new();
    std::io::stdin().lock().read_line(&mut command).expect("read CONNECT");
    let parts: Vec<_> = command.split_whitespace().collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "CONNECT");
    let server_id = EndpointId::from_str(parts[1]).expect("server ID");
    let socket = SocketAddr::from_str(parts[2]).expect("numeric loopback socket");

    if reconnect {
        let connection = connect_peer(&endpoint, server_id, socket).await;
        offer(&endpoint, &connection, false).await;
        connection.close(0u8.into(), b"simulated broken network path");
        // A new connection needs a new call ID, not an implicit continuation.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let connection = connect_peer(&endpoint, server_id, socket).await;
    offer(&endpoint, &connection, true).await;
    println!("CHILD_PASS client real_quic=1 reconnect={}", u8::from(reconnect));
    connection.close(0u8.into(), b"completed and acknowledged");
    endpoint.close().await;
}

struct Process {
    child: Child,
    stdout: Receiver<String>,
}

impl Process {
    fn spawn(test_name: &str, role: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("current test binary"))
            .arg("--exact")
            .arg(test_name)
            .arg("--nocapture")
            .env("AKULA_CALL_QUIC_CHILD", role)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn separate OS process");
        let stdout = child.stdout.take().expect("child stdout");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, stdout: rx }
    }

    fn marker(&self, marker: &str) -> String {
        for _ in 0..40 {
            let line = self.stdout.recv_timeout(WAIT).expect("child stalled");
            if let Some(index) = line.find(marker) {
                return line[index..].trim().to_owned();
            }
        }
        panic!("expected child marker: {marker}");
    }

    fn command(&mut self, message: &str) {
        writeln!(
            self.child.stdin.as_mut().expect("child stdin"),
            "{message}"
        )
        .expect("send test-selected public ID to child");
        self.child.stdin.as_mut().expect("child stdin").flush().expect("flush command");
    }

    fn completed(&mut self) {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("check child exit") {
                assert!(status.success(), "child process failed: {status}");
                return;
            }
            assert!(start.elapsed() < WAIT, "child process did not finish");
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run_two_processes(test_name: &str, reconnect: bool) {
    match std::env::var("AKULA_CALL_QUIC_CHILD").as_deref() {
        Ok("server") => {
            let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime");
            runtime.block_on(tokio::time::timeout(WAIT, server(reconnect)))
                .expect("server deadline");
            return;
        }
        Ok("client") => {
            let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime");
            runtime.block_on(tokio::time::timeout(WAIT, client(reconnect)))
                .expect("client deadline");
            return;
        }
        Ok(other) => panic!("unexpected laboratory child role: {other}"),
        Err(_) => {}
    }

    let mut server = Process::spawn(test_name, "server");
    let ready = server.marker("READY ");
    let parts: Vec<_> = ready.split_whitespace().collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "READY");
    let mut client = Process::spawn(test_name, "client");
    let client_ready = client.marker("READY ");
    let client_parts: Vec<_> = client_ready.split_whitespace().collect();
    assert_eq!(client_parts.len(), 2);
    server.command(&format!("EXPECT {}", client_parts[1]));
    client.command(&format!("CONNECT {} {}", parts[1], parts[2]));

    assert!(server.marker("CHILD_PASS").contains("real_quic=1"));
    assert!(client.marker("CHILD_PASS").contains("real_quic=1"));
    server.completed();
    client.completed();
    println!(
        "AKULA_CALL_REAL_QUIC_TWO_PROCESS_PASS fresh_call={} transport_loss={}",
        1,
        u8::from(reconnect)
    );
}

#[test]
fn two_os_processes_exchange_offer_ring_accept_and_end() {
    run_two_processes(
        "two_os_processes_exchange_offer_ring_accept_and_end",
        false,
    );
}

#[test]
fn two_os_processes_close_on_break_and_reconnect_with_fresh_call() {
    run_two_processes(
        "two_os_processes_close_on_break_and_reconnect_with_fresh_call",
        true,
    );
}
