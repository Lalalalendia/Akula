#![cfg(feature = "call-voice-quic-lab")]
//! AK-CALL-02: two actual OS processes and genuine Iroh/QUIC TLS exchange
//! bundled-libopus encoded SYNTHETIC PCM in both directions. This is only a
//! transport/codec interoperability test, not a microphone or user voice call.
//! Identity and a fresh call ID are supplied by a LOCAL TEST CONTROLLER, NOT
//! an OS pin, signed roster, user acceptance or authorization service.
//! The reliable QUIC bidirectional stream is deliberate for this *smoke*;
//! it does not benchmark real-time DATAGRAM/loss, latency, jitter or relay.

use akula_staya_integration::call_voice_codec_lab::{
    CodecLabError, DecodedAudio, OpusVoiceReceiver, OpusVoiceSender,
};
use aquilum_core::{
    p2p_offer_lab::PeerBinding,
    sync::DeviceId,
    voice_jitter_lab::{HEADER_BYTES, MAX_WIRE_BYTES, PeerPolicy, SAMPLES_PER_FRAME, VoiceError},
};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, endpoint::presets};
use std::{
    io::{BufRead, BufReader, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    process::{Child, Command, Stdio},
    str::FromStr,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const ALPN: &[u8] = b"akula/call-voice-codec/quic-lab/1";
const DEADLINE: Duration = Duration::from_secs(30);
const WAIT: Duration = Duration::from_secs(65);
const FRAMES: usize = 8;
const MAX_BATCH_BYTES: usize = FRAMES * (MAX_WIRE_BYTES + 2);

fn peer(device: u128, endpoint: EndpointId) -> PeerBinding {
    PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_u128(device)),
        endpoint_id: *endpoint.as_bytes(),
    }
}

fn pcm_frame(seed: usize, tick: usize) -> [i16; SAMPLES_PER_FRAME] {
    let mut samples = [0i16; SAMPLES_PER_FRAME];
    for (index, sample) in samples.iter_mut().enumerate() {
        let time = (tick * SAMPLES_PER_FRAME + index) as f64 / 48_000.0;
        let tone = (2.0 * std::f64::consts::PI * (330 + seed * 70) as f64 * time).sin();
        *sample = (tone * 15_000.0) as i16;
    }
    samples
}

/// Exact 16-bit bounded envelope; QUIC/TLS handles transport confidentiality.
/// These length fields do not authorize content or identify the sender.
fn push_frame(out: &mut Vec<u8>, wire: &[u8]) {
    assert!((HEADER_BYTES..=MAX_WIRE_BYTES).contains(&wire.len()));
    assert!(wire.len() <= u16::MAX as usize);
    out.extend_from_slice(&(wire.len() as u16).to_be_bytes());
    out.extend_from_slice(wire);
    assert!(out.len() <= MAX_BATCH_BYTES, "bounded frame batch");
}

fn next_frame<'a>(bytes: &'a [u8], cursor: &mut usize) -> &'a [u8] {
    let length_end = (*cursor).checked_add(2).expect("length offset");
    assert!(length_end <= bytes.len(), "short frame prefix");
    let len = u16::from_be_bytes([bytes[*cursor], bytes[*cursor + 1]]) as usize;
    assert!(
        (HEADER_BYTES..=MAX_WIRE_BYTES).contains(&len),
        "oversized frame"
    );
    let end = length_end.checked_add(len).expect("frame offset");
    assert!(end <= bytes.len(), "truncated encoded frame");
    *cursor = end;
    &bytes[length_end..end]
}

fn encode_frames(call_id: [u8; 16], seed: usize) -> Vec<u8> {
    let mut sender = OpusVoiceSender::new(call_id, 0).expect("real Opus encoder");
    assert!(sender.is_muted());
    // Test controller explicitly activates synthetic input, NOT microphone.
    sender.set_muted(false).expect("synthetic test consent");
    let mut out = Vec::new();
    for tick in 0..FRAMES {
        let wire = sender
            .encode_frame(&pcm_frame(seed, tick))
            .expect("real Opus software encoding");
        push_frame(&mut out, &wire);
    }
    sender.close();
    out
}

fn decode_frames(bytes: &[u8], call_id: [u8; 16], remote: PeerBinding) -> usize {
    let mut receiver = OpusVoiceReceiver::new(call_id, remote).expect("real Opus decoder");
    let mut cursor = 0;
    let mut nonzero = 0usize;
    for tick in 0..FRAMES {
        let wire = next_frame(bytes, &mut cursor);
        receiver
            .receive(wire, remote, PeerPolicy::Allowed, 100 + tick as u64 * 20)
            .expect("admit authenticated, correct-session Opus");
        match receiver
            .poll(160 + tick as u64 * 20)
            .expect("actual Opus decode")
        {
            DecodedAudio::Pcm {
                tick: actual,
                samples,
                concealed: false,
            } => {
                assert_eq!(actual, tick as u64);
                nonzero += samples.iter().filter(|x| **x != 0).count();
            }
            other => panic!("expected real decoded PCM, got {other:?}"),
        }
    }
    assert_eq!(cursor, bytes.len(), "no trailing payload");
    assert!(nonzero > 1000, "decoded audio is not all zero");
    assert_eq!(receiver.metrics().played_opus_ticks, FRAMES as u64);
    assert_eq!(receiver.metrics().conceal_ticks, 0);
    receiver.close();
    nonzero
}

fn negative_frames(call_id: [u8; 16]) -> Vec<u8> {
    let mut wrong_call = call_id;
    wrong_call[0] ^= 0xff;
    let mut out = Vec::new();
    let mut wrong = OpusVoiceSender::new(wrong_call, 0).unwrap();
    wrong.set_muted(false).unwrap();
    push_frame(&mut out, &wrong.encode_frame(&pcm_frame(3, 0)).unwrap());
    let mut valid = OpusVoiceSender::new(call_id, 0).unwrap();
    valid.set_muted(false).unwrap();
    push_frame(&mut out, &valid.encode_frame(&pcm_frame(4, 0)).unwrap());
    out
}

fn inspect_revoke(bytes: &[u8], call_id: [u8; 16], remote: PeerBinding) {
    let mut receiver = OpusVoiceReceiver::new(call_id, remote).unwrap();
    let mut cursor = 0;
    let wrong_call = next_frame(bytes, &mut cursor);
    assert!(matches!(
        receiver.receive(wrong_call, remote, PeerPolicy::Allowed, 100),
        Err(CodecLabError::Voice(VoiceError::WrongCall))
    ));
    assert_eq!(receiver.queued_ticks(), 0);
    let correct = next_frame(bytes, &mut cursor);
    receiver
        .receive(correct, remote, PeerPolicy::Allowed, 101)
        .expect("genuine valid frame");
    assert_eq!(receiver.queued_ticks(), 1);
    // This is a simulated local policy change from an external controller,
    // not a live OS pin notification or remote-initiated mute.
    assert!(matches!(
        receiver.receive(correct, remote, PeerPolicy::Revoked, 102),
        Err(CodecLabError::Voice(VoiceError::Revoked))
    ));
    assert_eq!(receiver.queued_ticks(), 0);
    assert!(matches!(
        receiver.receive(correct, remote, PeerPolicy::Allowed, 103),
        Err(CodecLabError::Voice(VoiceError::Closed))
    ));
    assert_eq!(cursor, bytes.len());
}

fn new_endpoint() -> impl std::future::Future<Output = Endpoint> {
    async {
        Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .expect("ephemeral direct-Iroh lab endpoint")
    }
}

fn supplied_call(value: &str) -> [u8; 16] {
    let uuid = Uuid::from_str(value).expect("test controller call UUID");
    assert!(!uuid.is_nil());
    *uuid.as_bytes()
}

async fn accept_peer(endpoint: &Endpoint, expected: EndpointId) -> iroh::endpoint::Connection {
    let incoming = tokio::time::timeout(DEADLINE, endpoint.accept())
        .await
        .expect("Iroh incoming deadline")
        .expect("incoming connection");
    let handshake = incoming.accept().expect("QUIC TLS handshake");
    let connection = tokio::time::timeout(DEADLINE, handshake)
        .await
        .expect("TLS deadline")
        .expect("authenticated QUIC TLS");
    assert_eq!(
        connection.remote_id(),
        expected,
        "independently pinned test key"
    );
    connection
}

async fn server_unreliable_datagram(connection: &iroh::endpoint::Connection, call_id: [u8; 16]) {
    let remote = peer(1, connection.remote_id());
    let wire = tokio::time::timeout(DEADLINE, connection.read_datagram())
        .await
        .expect("unreliable Opus arrival timeout")
        .expect("read encrypted QUIC datagram");
    let mut receiver = OpusVoiceReceiver::new(call_id, remote).unwrap();
    receiver
        .receive(&wire, remote, PeerPolicy::Allowed, 100)
        .expect("bound datagram for independently selected peer and call");
    assert!(matches!(
        receiver.poll(160).expect("decode real Opus"),
        DecodedAudio::Pcm {
            concealed: false,
            ..
        }
    ));
    receiver.close();

    let mut sender = OpusVoiceSender::new(call_id, 0).unwrap();
    sender
        .set_muted(false)
        .expect("test controller synthetic audio");
    let reply = sender
        .encode_frame(&pcm_frame(2, 0))
        .expect("real Opus encoding for QUIC datagram");
    let max = connection
        .max_datagram_size()
        .expect("peer must negotiate QUIC DATAGRAM capability");
    assert!(reply.len() <= max, "do not fragment time-critical voice");
    connection
        .send_datagram(reply.into())
        .expect("queue one bounded encrypted reply datagram");

    // Reliable acknowledgement only coordinates the TEST. Voice itself uses
    // one unreliable, unordered QUIC DATAGRAM in each direction.
    let (mut ack_send, mut ack_receive) = tokio::time::timeout(DEADLINE, connection.accept_bi())
        .await
        .expect("test acknowledgement deadline")
        .expect("test acknowledgement stream");
    let ack = tokio::time::timeout(DEADLINE, ack_receive.read_to_end(64))
        .await
        .expect("test acknowledgement read deadline")
        .expect("bounded test acknowledgement");
    assert_eq!(ack, b"VOICE_DATAGRAM_RECEIVED");
    ack_send.write_all(b"DGRAM_DONE").await.unwrap();
    ack_send.finish().unwrap();
    tokio::time::timeout(DEADLINE, ack_send.stopped())
        .await
        .expect("acknowledgement delivery deadline")
        .expect("acknowledgement completed");
}

async fn client_unreliable_datagram(connection: &iroh::endpoint::Connection, call_id: [u8; 16]) {
    let mut sender = OpusVoiceSender::new(call_id, 0).unwrap();
    sender
        .set_muted(false)
        .expect("test controller synthetic audio");
    let wire = sender
        .encode_frame(&pcm_frame(1, 0))
        .expect("real Opus encoding for QUIC datagram");
    let max = connection
        .max_datagram_size()
        .expect("peer must negotiate QUIC DATAGRAM capability");
    assert!(wire.len() <= max, "do not fragment time-critical voice");
    connection
        .send_datagram(wire.into())
        .expect("queue one bounded encrypted Opus datagram");
    let response = tokio::time::timeout(DEADLINE, connection.read_datagram())
        .await
        .expect("reply datagram arrival timeout")
        .expect("read encrypted Opus response datagram");
    let remote = peer(2, connection.remote_id());
    let mut receiver = OpusVoiceReceiver::new(call_id, remote).unwrap();
    receiver
        .receive(&response, remote, PeerPolicy::Allowed, 100)
        .expect("authenticated reply bound to selected call");
    assert!(matches!(
        receiver.poll(160).expect("decode returned Opus"),
        DecodedAudio::Pcm {
            concealed: false,
            ..
        }
    ));
    receiver.close();
    let (mut ack_send, mut ack_receive) = tokio::time::timeout(DEADLINE, connection.open_bi())
        .await
        .expect("test acknowledgement stream deadline")
        .expect("test acknowledgement stream");
    ack_send
        .write_all(b"VOICE_DATAGRAM_RECEIVED")
        .await
        .unwrap();
    ack_send.finish().unwrap();
    let ack = tokio::time::timeout(DEADLINE, ack_receive.read_to_end(64))
        .await
        .expect("test acknowledgement deadline")
        .expect("bounded test acknowledgement");
    assert_eq!(ack, b"DGRAM_DONE");
}

async fn server(negative: bool, datagrams: bool) {
    let endpoint = new_endpoint().await;
    let socket = endpoint
        .bound_sockets()
        .into_iter()
        .find(SocketAddr::is_ipv4)
        .or_else(|| endpoint.bound_sockets().into_iter().next())
        .expect("bound QUIC UDP");
    let loopback = match socket {
        SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    println!(
        "READY {} {}",
        endpoint.id(),
        SocketAddr::new(loopback, socket.port())
    );
    std::io::stdout().flush().expect("flush READY");

    let mut command = String::new();
    std::io::stdin().lock().read_line(&mut command).unwrap();
    let parts: Vec<_> = command.split_whitespace().collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "EXPECT");
    let expected = EndpointId::from_str(parts[1]).expect("pinned test peer");
    let call_id = supplied_call(parts[2]);

    let connection = accept_peer(&endpoint, expected).await;
    if datagrams {
        server_unreliable_datagram(&connection, call_id).await;
        println!("CHILD_PASS server encrypted_quic=1 real_opus=1 datagram=1");
        connection.close(0u8.into(), b"unreliable synthetic voice smoke complete");
        endpoint.close().await;
        return;
    }
    let (mut reply, mut receive) = tokio::time::timeout(DEADLINE, connection.accept_bi())
        .await
        .expect("media bi-stream timeout")
        .expect("media bi-stream");
    let request = tokio::time::timeout(DEADLINE, receive.read_to_end(MAX_BATCH_BYTES))
        .await
        .expect("media request deadline")
        .expect("bounded synthetic PCM encoded batch");
    if negative {
        inspect_revoke(&request, call_id, peer(1, connection.remote_id()));
        reply
            .write_all(b"AKULA_VOICE_REVOKED")
            .await
            .expect("return revoked marker");
    } else {
        let samples = decode_frames(&request, call_id, peer(1, connection.remote_id()));
        assert!(samples > 1000);
        reply
            .write_all(&encode_frames(call_id, 2))
            .await
            .expect("send real Opus response");
    }
    reply.finish().expect("finish response");
    tokio::time::timeout(DEADLINE, reply.stopped())
        .await
        .expect("receiver acknowledgement deadline")
        .expect("response finished");
    println!(
        "CHILD_PASS server encrypted_quic=1 real_opus=1 negative={}",
        u8::from(negative)
    );
    connection.close(0u8.into(), b"synthetic voice lab complete");
    endpoint.close().await;
}

async fn client(negative: bool, datagrams: bool) {
    let endpoint = new_endpoint().await;
    println!("READY {}", endpoint.id());
    std::io::stdout().flush().expect("flush READY");
    let mut command = String::new();
    std::io::stdin().lock().read_line(&mut command).unwrap();
    let parts: Vec<_> = command.split_whitespace().collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], "CONNECT");
    let expected = EndpointId::from_str(parts[1]).expect("trusted server key");
    let socket = SocketAddr::from_str(parts[2]).expect("numeric loopback QUIC socket");
    let call_id = supplied_call(parts[3]);
    let address = EndpointAddr::new(expected).with_ip_addr(socket);
    let connection = tokio::time::timeout(DEADLINE, endpoint.connect(address, ALPN))
        .await
        .expect("direct connection deadline")
        .expect("TLS/QUIC connection");
    assert_eq!(connection.remote_id(), expected, "wrong remote TLS key");
    if datagrams {
        client_unreliable_datagram(&connection, call_id).await;
        println!("CHILD_PASS client encrypted_quic=1 real_opus=1 datagram=1");
        connection.close(0u8.into(), b"unreliable synthetic voice smoke complete");
        endpoint.close().await;
        return;
    }
    let (mut send, mut response) = tokio::time::timeout(DEADLINE, connection.open_bi())
        .await
        .expect("media send stream deadline")
        .expect("media send stream");
    let request = if negative {
        negative_frames(call_id)
    } else {
        encode_frames(call_id, 1)
    };
    send.write_all(&request)
        .await
        .expect("send real Opus frames");
    send.finish().expect("finish test media");
    let reply = tokio::time::timeout(DEADLINE, response.read_to_end(MAX_BATCH_BYTES))
        .await
        .expect("synthetic response deadline")
        .expect("bounded server response");
    if negative {
        assert_eq!(reply, b"AKULA_VOICE_REVOKED");
    } else {
        let nonzero = decode_frames(&reply, call_id, peer(2, connection.remote_id()));
        assert!(nonzero > 1000);
    }
    println!(
        "CHILD_PASS client encrypted_quic=1 real_opus=1 negative={}",
        u8::from(negative)
    );
    connection.close(0u8.into(), b"synthetic voice lab complete");
    endpoint.close().await;
}

struct Process {
    child: Child,
    stdout: Receiver<String>,
}

impl Process {
    fn spawn(test_name: &str, role: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("test runner path"))
            .arg("--exact")
            .arg(test_name)
            .arg("--nocapture")
            .env("AKULA_VOICE_QUIC_CHILD", role)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn independent OS process");
        let output = child.stdout.take().expect("child output");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, stdout: rx }
    }

    fn marker(&self, prefix: &str) -> String {
        for _ in 0..40 {
            let line = self.stdout.recv_timeout(WAIT).expect("child stalled");
            if let Some(position) = line.find(prefix) {
                return line[position..].trim().to_string();
            }
        }
        panic!("missing child marker: {prefix}");
    }

    fn command(&mut self, line: &str) {
        writeln!(self.child.stdin.as_mut().expect("child stdin"), "{line}")
            .expect("send test controller command");
        self.child.stdin.as_mut().unwrap().flush().unwrap();
    }

    fn completed(&mut self) {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("process status") {
                assert!(status.success(), "child test did not exit successfully");
                return;
            }
            assert!(started.elapsed() < WAIT, "child exceeded test deadline");
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

fn two_processes(test_name: &str, negative: bool, datagrams: bool) {
    match std::env::var("AKULA_VOICE_QUIC_CHILD").as_deref() {
        Ok("server") => {
            let runtime = tokio::runtime::Runtime::new().expect("Tokio multi-thread runtime");
            runtime.block_on(async {
                tokio::time::timeout(WAIT, server(negative, datagrams))
                    .await
                    .expect("server overall deadline");
            });
            return;
        }
        Ok("client") => {
            let runtime = tokio::runtime::Runtime::new().expect("Tokio multi-thread runtime");
            runtime.block_on(async {
                tokio::time::timeout(WAIT, client(negative, datagrams))
                    .await
                    .expect("client overall deadline");
            });
            return;
        }
        Ok(other) => panic!("unexpected child role: {other}"),
        Err(_) => {}
    }
    // Selected by local test runner, not from untrusted remote metadata.
    let call_id = Uuid::new_v4();
    let mut server = Process::spawn(test_name, "server");
    let ready = server.marker("READY ");
    let server_fields: Vec<_> = ready.split_whitespace().collect();
    assert_eq!(server_fields.len(), 3);
    let mut client = Process::spawn(test_name, "client");
    let client_ready = client.marker("READY ");
    let client_fields: Vec<_> = client_ready.split_whitespace().collect();
    assert_eq!(client_fields.len(), 2);
    server.command(&format!("EXPECT {} {call_id}", client_fields[1]));
    client.command(&format!(
        "CONNECT {} {} {call_id}",
        server_fields[1], server_fields[2]
    ));
    assert!(
        server.marker("CHILD_PASS").contains("encrypted_quic=1"),
        "server must complete real media"
    );
    assert!(
        client.marker("CHILD_PASS").contains("real_opus=1"),
        "client must complete real media"
    );
    server.completed();
    client.completed();
    if datagrams {
        println!("AKULA_CALL_VOICE_DATAGRAM_TWO_PROCESS_PASS actual_quic=1 actual_libopus=1");
    } else {
        println!(
            "AKULA_CALL_VOICE_TWO_PROCESS_PASS actual_quic=1 actual_libopus=1 negative={}",
            u8::from(negative)
        );
    }
}

#[test]
fn two_os_processes_exchange_synthetic_opus_in_both_directions() {
    two_processes(
        "two_os_processes_exchange_synthetic_opus_in_both_directions",
        false,
        false,
    );
}

#[test]
fn two_os_processes_reject_old_call_id_and_close_on_revoke() {
    two_processes(
        "two_os_processes_reject_old_call_id_and_close_on_revoke",
        true,
        false,
    );
}

#[test]
fn two_os_processes_exchange_synthetic_opus_over_quic_datagrams() {
    two_processes(
        "two_os_processes_exchange_synthetic_opus_over_quic_datagrams",
        false,
        true,
    );
}
