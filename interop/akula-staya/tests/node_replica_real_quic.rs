//! NODE-E2E-02a: send a previously verified, offline-owner Node replica
//! through the *existing* Akula OfferSession -> real Staya Iroh QUIC/Bao path.
//!
//! Explicit, disposable LAB ONLY: no real device enrollment, live roster,
//! production Node daemon, server credential custody, File Gate or source write.
//! A trusted test operator binds an ephemeral device key to the Iroh EndpointId.
//! The missing signed DeviceId <-> EndpointId link is a release blocker.
#![cfg(all(target_os = "linux", feature = "node-replica-transport-lab"))]

use akula_staya_integration::{prepare_staya_batch, verify_staya_invitation};
use aquilum_core::{
    node_replica_auth_lab::{
        DeviceReadProof, IndependentReadTrust, OwnerReadGrant, ReadDenied, device_challenge_claim,
        owner_claim, read_with_device_proof, signed_grant_digest,
    },
    node_replica_lab::{OperatorFixtureAdmission, stage_operator_fixture},
    p2p_offer_lab::{DecisionAction, OfferV1, OfferedFile, PeerBinding, SCHEMA},
    sync::{DeviceId, DocumentId, RevisionId, VaultId},
};
use iroh::{EndpointAddr, EndpointId};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    str::FromStr,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(90);

struct StayaProcess {
    child: Child,
    lines: Receiver<String>,
}

impl StayaProcess {
    fn spawn(bin: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("launch pinned real Staya preview binary");
        let stdout = child.stdout.take().expect("Staya stdout");
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    fn line(&self) -> String {
        self.lines
            .recv_timeout(WAIT)
            .expect("timed out waiting for Staya line")
    }

    fn send_line(&mut self, command: &str) {
        self.child
            .stdin
            .as_mut()
            .expect("Staya stdin")
            .write_all(format!("{command}\n").as_bytes())
            .expect("send test-only local command");
    }

    fn send(&mut self, command: &str) {
        self.send_line(command);
        drop(self.child.stdin.take());
    }

    fn finish(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("Staya child state") {
                return status;
            }
            assert!(started.elapsed() < WAIT, "Staya child did not exit");
            thread::sleep(Duration::from_millis(30));
        }
    }
}

impl Drop for StayaProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn decode_hex(input: &str) -> Vec<u8> {
    assert!(!input.is_empty() && input.len() <= 8192 && input.len().is_multiple_of(2));
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
        .collect()
}

fn store_replica_and_sign_grant(
    base: &Path,
    device: &Ed25519KeyPair,
    owner: &Ed25519KeyPair,
) -> (
    PathBuf,
    Vec<u8>,
    OwnerReadGrant,
    IndependentReadTrust,
    DeviceReadProof,
) {
    let source = base.join("original-vault");
    let state = base.join("node-owned-state");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();

    let original = b"# Akula real Staya QUIC\nOffline original; Node replica stays.\n".to_vec();
    fs::write(source.join("note.md"), &original).unwrap();
    let vault_id = VaultId::from_uuid(Uuid::from_u128(0x333333));
    let document_id = DocumentId::from_uuid(Uuid::from_u128(0x444444));
    let revision_id = RevisionId::from_uuid(Uuid::from_u128(0x555555));
    let expected_blake3 = blake3::hash(&original).to_hex().to_string();
    let admission = OperatorFixtureAdmission {
        vault_id,
        document_id,
        revision_id,
        filename: "note.md".to_owned(),
        expected_blake3: expected_blake3.clone(),
    };
    stage_operator_fixture(&source, &state, &admission)
        .expect("commit through original Sync DAG, not a new writer");

    // The source is deliberately unavailable before a receiver can GET it.
    fs::remove_file(source.join("note.md")).unwrap();
    assert_eq!(fs::read_dir(&source).unwrap().count(), 0);

    let mut grant = OwnerReadGrant {
        vault_id,
        document_id,
        revision_id,
        expected_blake3,
        filename: "note.md".to_owned(),
        node_identity: [0x71u8; 32],
        device_id: DeviceId::from_uuid(Uuid::from_u128(0x666666)),
        device_public_key: device.public_key().as_ref().try_into().unwrap(),
        grant_epoch: 7,
        owner_signature: [0u8; 64],
    };
    grant.owner_signature = owner
        .sign(&owner_claim(&grant).unwrap())
        .as_ref()
        .try_into()
        .unwrap();
    let trust = IndependentReadTrust {
        trusted_vault_id: vault_id,
        trusted_owner_public_key: owner.public_key().as_ref().try_into().unwrap(),
        trusted_node_identity: grant.node_identity,
        witnessed_current_epoch: grant.grant_epoch,
        witnessed_exact_grant_digest: signed_grant_digest(&grant).unwrap(),
        current_session_challenge: [0x91u8; 32],
    };
    let proof = DeviceReadProof {
        device_signature: device
            .sign(&device_challenge_claim(&grant, trust.current_session_challenge).unwrap())
            .as_ref()
            .try_into()
            .unwrap(),
    };
    (state, original, grant, trust, proof)
}

#[test]
fn offline_node_replica_reaches_another_real_iroh_process_only_after_authorization() {
    let bin = PathBuf::from(
        std::env::var_os("STAYA_PREVIEW_BIN").expect("CI must build pinned real Staya binary"),
    );
    assert!(bin.is_file());

    let temp = tempfile::tempdir().unwrap();
    let owner = Ed25519KeyPair::from_seed_unchecked(&[12u8; 32]).unwrap();
    let device = Ed25519KeyPair::from_seed_unchecked(&[21u8; 32]).unwrap();
    let (state, original, grant, trust, proof) =
        store_replica_and_sign_grant(temp.path(), &device, &owner);
    let export = temp.path().join("object_00.bin");

    // A revoked epoch and another device NEVER release any content to Staya.
    let mut revoked = trust.clone();
    revoked.witnessed_current_epoch += 1;
    assert_eq!(
        read_with_device_proof(&state, &grant, &revoked, &proof),
        Err(ReadDenied)
    );
    let outsider = Ed25519KeyPair::from_seed_unchecked(&[32u8; 32]).unwrap();
    let outsider_proof = DeviceReadProof {
        device_signature: outsider
            .sign(&device_challenge_claim(&grant, trust.current_session_challenge).unwrap())
            .as_ref()
            .try_into()
            .unwrap(),
    };
    assert_eq!(
        read_with_device_proof(&state, &grant, &trust, &outsider_proof),
        Err(ReadDenied)
    );
    assert!(
        !export.exists(),
        "no materialized plaintext on any denied grant"
    );

    let bytes = read_with_device_proof(&state, &grant, &trust, &proof)
        .expect("test operator provided independently pinned current owner grant");
    assert_eq!(bytes, original);
    // Staya's current preview provider accepts local paths only. This
    // *disposable plaintext materialization* is NOT a production NodeStore
    // exporter or a secure streaming adapter.
    fs::write(&export, &bytes).unwrap();

    let mut receiver = StayaProcess::spawn(
        &bin,
        &[
            "receive-batch",
            "--lab-request-unlisted",
            "--lab-probe-before-arm",
        ],
    );
    let receiver_id = receiver.line();
    let receiver_id = receiver_id
        .strip_prefix("PEER_ID ")
        .expect("live Iroh receiver");
    let endpoint = EndpointId::from_str(receiver_id).unwrap();
    let mut provider = StayaProcess::spawn(
        &bin,
        &["serve-batch-gated", receiver_id, export.to_str().unwrap()],
    );
    let invitation = provider.line();
    let fields: Vec<&str> = invitation.split_whitespace().collect();
    assert_eq!(fields.first(), Some(&"CONNECT_BATCH"));
    assert_eq!(fields.len(), 7, "single physical verified Node replica");

    let ticket: EndpointAddr = postcard::from_bytes(&decode_hex(fields[1])).unwrap();
    assert!(!ticket.is_empty());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let sender = PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::new_v4()),
        endpoint_id: *ticket.id.as_bytes(),
    };
    let recipient = PeerBinding {
        device_id: grant.device_id,
        endpoint_id: *endpoint.as_bytes(),
    };
    let offer = OfferV1 {
        schema: SCHEMA,
        offer_id: Uuid::new_v4(),
        nonce: [0x61u8; 16],
        sender,
        receiver: recipient,
        created_unix: now,
        expires_unix: now + 120,
        files: vec![OfferedFile {
            label: "Node replica (disposable).md".to_owned(),
            bytes: bytes.len() as u64,
            blake3_root: *blake3::hash(&bytes).as_bytes(),
        }],
    };
    let decision = offer
        .receiver_decision(DecisionAction::Accept, vec![0], now, sender, recipient)
        .expect("exact selected root");
    let prepared = prepare_staya_batch(offer, &decision, sender, recipient, now + 1)
        .expect("real Akula -> Staya queue");
    assert_eq!(prepared.queue.entries().len(), 1);
    assert_eq!(prepared.queue.entries()[0].label, "object_00.bin");
    verify_staya_invitation(&prepared, &invitation, sender.endpoint_id)
        .expect("real endpoint ticket, size and root match selected replica");

    // The actual Iroh receiver exists, but a GET before sender-local Akula
    // consent is denied by the real Staya provider.
    receiver.send_line(&invitation);
    assert_eq!(receiver.line(), "PREARM_DENIED=true");
    provider.send_line(&format!("AUTHORIZE {} {}", fields[2], receiver_id));
    assert_eq!(provider.line(), "PROVIDER_ARMED");

    let mut unknown_peer = StayaProcess::spawn(&bin, &["receive-batch"]);
    assert!(unknown_peer.line().starts_with("PEER_ID "));
    unknown_peer.send(&invitation);
    assert!(
        !unknown_peer.finish().success(),
        "unlisted real QUIC key must not GET"
    );
    assert!(
        !unknown_peer
            .lines
            .try_iter()
            .any(|line| line.starts_with("STAGED_QUEUE "))
    );

    receiver.send("CONTINUE");
    let line = receiver.line();
    let path = line
        .strip_prefix("STAGED_QUEUE 0 ")
        .expect("real Bao staged replica");
    assert!(path.contains("NOT_APPLIED=true"));
    let physical = PathBuf::from(path.split(" ROOT=").next().unwrap());
    assert_eq!(fs::read(&physical).unwrap(), original);
    assert_eq!(
        *blake3::hash(&fs::read(&physical).unwrap()).as_bytes(),
        prepared.queue.entries()[0].manifest.id.0
    );
    assert_eq!(receiver.line(), "STY_G1K_UNLISTED_GET_DENIED=true");
    let result = receiver.line();
    assert!(result.starts_with("QUEUE_COMPLETE COUNT=1 COMMITMENT="));
    assert!(result.contains("VERIFIED_ITEMS=1/1"));
    assert!(result.contains("NOT_APPLIED=true"));
    assert!(receiver.finish().success());
    drop(provider.child.stdin.take());
    assert!(provider.finish().success());
    fs::remove_dir_all(physical.parent().unwrap()).expect("remove only Staya temp stage");

    println!(
        "NODE_E2E_02A_REAL_STAYA=true offline_original=true signed_read=true \
             provider_prearm_denied=true outsider_denied=true bao_verified=true \
             enrolled_device=false production_vps=false"
    );
}
