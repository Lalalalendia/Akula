//! Real Iroh/QUIC two-OS-process transfer controlled by an Akula consent
//! decision and the actual Staya QueueSelection. Only disposable test files.
//! This is not proof of trusted long-lived Device enrollment or File Gate.

use akula_inbox_ledger_lab::{ExpectedTransfer, InboxLedger, LedgerError, LocalStatus, Transport};
use akula_staya_integration::{PreparedBatch, prepare_staya_batch, verify_staya_invitation};
use aquilum_core::{
    p2p_offer_lab::{DecisionAction, OfferV1, OfferedFile, PeerBinding, SCHEMA},
    sync::DeviceId,
};
use iroh::{EndpointAddr, EndpointId};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    str::FromStr,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(90);

struct OwnedProcess {
    child: Child,
    output: Receiver<String>,
}

impl OwnedProcess {
    fn spawn(binary: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("launch actual Staya executable in independent process");
        let stdout = child.stdout.take().expect("Staya stdout pipe");
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, output }
    }

    fn line(&self) -> String {
        self.output
            .recv_timeout(WAIT)
            .expect("Staya process did not emit expected protocol line")
    }

    fn send_line(&mut self, value: &str) {
        self.child
            .stdin
            .as_mut()
            .expect("child input")
            .write_all(format!("{value}\n").as_bytes())
            .expect("send exact Akula-approved Staya command");
    }

    fn send(&mut self, value: &str) {
        self.send_line(value);
        drop(self.child.stdin.take());
    }

    fn finish(&mut self) -> ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("query child status") {
                return status;
            }
            assert!(start.elapsed() < WAIT, "Staya child did not terminate");
            thread::sleep(Duration::from_millis(30));
        }
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct SyntheticFiles(PathBuf);

impl SyntheticFiles {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("akula-staya-e2e-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).expect("private disposable test root");
        Self(path)
    }

    fn file(&self, label: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(label);
        fs::write(&path, bytes).expect("synthetic test source");
        path
    }
}

impl Drop for SyntheticFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Vec<u8> {
    assert!(!value.is_empty() && value.len() <= 8192 && value.len().is_multiple_of(2));
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex utf8");
            u8::from_str_radix(text, 16).expect("hex byte")
        })
        .collect()
}

fn source(label: &str, payload: &[u8]) -> OfferedFile {
    OfferedFile {
        label: label.into(),
        bytes: payload.len() as u64,
        blake3_root: *blake3::hash(payload).as_bytes(),
    }
}

/// A stable logical ID for one explicitly selected, independently checked
/// synthetic offer item. This is not a remote sender authorization.
fn ledger_item(
    offer: &OfferV1,
    prepared: &PreparedBatch,
    slot: usize,
    sender: PeerBinding,
    receiver: PeerBinding,
) -> ExpectedTransfer {
    let index = prepared.offer_indexes[slot];
    let selected = prepared.queue.entries()[slot].manifest;
    let source = &offer.files[index];
    assert_eq!(source.bytes, selected.size);
    assert_eq!(source.blake3_root, selected.id.0);
    let mut hasher = blake3::Hasher::new_derive_key("Akula P2P local received item id v1");
    hasher.update(offer.offer_id.as_bytes());
    hasher.update(&[u8::try_from(index).expect("bounded Akula offer index")]);
    hasher.update(&prepared.offer_commitment);
    hasher.update(&sender.endpoint_id);
    hasher.update(&receiver.endpoint_id);
    let commitment = hasher.finalize();
    let mut uuid_bytes = [0u8; 16];
    uuid_bytes.copy_from_slice(&commitment.as_bytes()[..16]);
    // A deterministic version-8 UUID derived from the accepted local offer.
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x80;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    ExpectedTransfer {
        id: Uuid::from_bytes(uuid_bytes),
        sender_key: sender.endpoint_id,
        recipient_key: receiver.endpoint_id,
        source: Transport::P2p,
        bytes: selected.size,
        blake3: selected.id.0,
    }
}

#[test]
fn akula_consent_to_real_staya_direct_quic_and_cold_bao_stage() {
    let binary = PathBuf::from(
        std::env::var_os("STAYA_PREVIEW_BIN").expect("CI must provide pinned real Staya binary"),
    );
    assert!(binary.is_file(), "real Staya binary is required");

    // The offer has THREE sources, but the receiver selects only 0 and 2.
    let sources = SyntheticFiles::new();
    let first = b"hello from Akula's approved object".to_vec();
    let forbidden = b"not selected by receiver".to_vec();
    let last = (0..(128 * 1024 + 17))
        .map(|i| ((i * 31 + 7) % 251) as u8)
        .collect::<Vec<_>>();
    let first_path = sources.file("object_00.bin", &first);
    let _unselected_path = sources.file("object_01.bin", &forbidden);
    let last_path = sources.file("object_02.bin", &last);

    // Two independent OS processes; the receiver creates a real Iroh key.
    let mut receiver = OwnedProcess::spawn(
        &binary,
        &[
            "receive-batch",
            "--lab-request-unlisted",
            "--lab-probe-before-arm",
        ],
    );
    let receiver_text = receiver.line();
    let receiver_id = receiver_text
        .strip_prefix("PEER_ID ")
        .expect("Staya receiver announced an actual Iroh EndpointId");
    let receiver_endpoint = EndpointId::from_str(receiver_id).expect("cryptographic public key");

    // Staya provider is restricted to the physically selected subset, not
    // every object in the offer. It will only serve the named receiver key.
    let first_arg = first_path.to_str().expect("test path utf8");
    let last_arg = last_path.to_str().expect("test path utf8");
    let mut provider = OwnedProcess::spawn(
        &binary,
        &["serve-batch-gated", receiver_id, first_arg, last_arg],
    );
    let invitation = provider.line();
    let fields: Vec<&str> = invitation.split_whitespace().collect();
    assert_eq!(fields.first(), Some(&"CONNECT_BATCH"));
    assert_eq!(
        fields.len(),
        10,
        "two explicit roots in real Staya invitation"
    );
    let endpoint_addr: EndpointAddr =
        postcard::from_bytes(&unhex(fields[1])).expect("decode real Iroh endpoint ticket");
    assert!(!endpoint_addr.is_empty(), "direct-only reachable transport");

    // Only the real Iroh public keys come from processes. DeviceIds here are
    // disposable test enrolment placeholders, NOT trusted production records.
    let sender = PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::new_v4()),
        endpoint_id: *endpoint_addr.id.as_bytes(),
    };
    let receiver_peer = PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::new_v4()),
        endpoint_id: *receiver_endpoint.as_bytes(),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall time")
        .as_secs();
    let offer = OfferV1 {
        schema: SCHEMA,
        offer_id: Uuid::new_v4(),
        nonce: [17; 16],
        sender,
        receiver: receiver_peer,
        created_unix: now,
        expires_unix: now + 120,
        files: vec![
            source("Договор — только отображение.txt", &first),
            source("Не выбран — не отправлять.txt", &forbidden),
            source("Архив — только отображение.bin", &last),
        ],
    };
    let decision = offer
        .receiver_decision(
            DecisionAction::Accept,
            vec![0, 2],
            now,
            sender,
            receiver_peer,
        )
        .expect("explicit bounded accepted subset");

    // A forged commitment must be denied *before* handing the ticket to
    // the actual Staya receiver process.
    let mut forged = decision.clone();
    forged.manifest_commitment = [0x70; 32];
    assert!(prepare_staya_batch(offer.clone(), &forged, sender, receiver_peer, now + 1).is_err());

    // This call crosses the real Akula -> real Staya library boundary.
    let prepared = prepare_staya_batch(offer.clone(), &decision, sender, receiver_peer, now + 1)
        .expect("validated Akula consent becomes a Staya queue, not a grant");
    assert_eq!(prepared.offer_indexes, [0, 2]);
    assert_eq!(prepared.queue.entries().len(), 2);
    assert_eq!(prepared.queue.entries()[0].label, "object_00.bin");
    assert_eq!(prepared.queue.entries()[1].label, "object_02.bin");
    assert_eq!(fields[2], hex(&prepared.queue.commitment()));
    assert_eq!(fields[3], "2");
    for (slot, object) in prepared.queue.entries().iter().enumerate() {
        let base = 4 + slot * 3;
        assert_eq!(fields[base], object.label);
        assert_eq!(fields[base + 1], hex(&object.manifest.id.0));
        assert_eq!(fields[base + 2], object.manifest.size.to_string());
        assert_ne!(
            object.manifest.id.0, offer.files[1].blake3_root,
            "unselected root may not be served"
        );
    }

    // The real bridge validates the original serialized Iroh ticket and
    // Staya ordered-root commitment before releasing it to the receiver.
    verify_staya_invitation(&prepared, &invitation, sender.endpoint_id)
        .expect("approved real Staya invitation must match live Akula offer");
    assert!(verify_staya_invitation(&prepared, &invitation, [0u8; 32]).is_err());
    let mut changed_invitation = fields
        .iter()
        .map(|field| (*field).to_owned())
        .collect::<Vec<_>>();
    changed_invitation[5] = hex(&offer.files[1].blake3_root);
    assert!(
        verify_staya_invitation(&prepared, &changed_invitation.join(" "), sender.endpoint_id)
            .is_err(),
        "inserting unselected content must fail before GET"
    );

    // Even the exact approved ephemeral receiver MUST fail before consent
    // is released to the provider, not only in the test controller.
    receiver.send_line(&invitation);
    assert_eq!(
        receiver.line(),
        "PREARM_DENIED=true",
        "real authorized Iroh key fetched before Akula approval"
    );

    // This command remains a trusted sender-local test seam: the real Akula
    // OfferSession decision and actual Staya invitation were verified above.
    // A production host must replace stdin with authenticated local IPC.
    provider.send_line(&format!("AUTHORIZE {} {}", fields[2], receiver_id));
    assert_eq!(
        provider.line(),
        "PROVIDER_ARMED",
        "Staya provider did not enforce explicit sender authorization"
    );

    // A separate, unlisted cryptographic receiver cannot GET bytes even
    // with the same endpoint ticket and otherwise valid invitation.
    let mut outsider = OwnedProcess::spawn(&binary, &["receive-batch"]);
    let outsider_id = outsider.line();
    assert!(outsider_id.starts_with("PEER_ID "));
    assert_ne!(outsider_id, receiver_text);
    outsider.send(&invitation);
    assert!(!outsider.finish().success(), "unlisted receiver must fail");
    assert!(
        !outsider
            .output
            .try_iter()
            .any(|line| line.starts_with("STAGED_QUEUE "))
    );

    // Real QUIC/Bao transfer begins only after verified Akula consent and
    // provider-side arming, using the original receiver key across attempts.
    receiver.send("CONTINUE");
    let mut staged_directories = Vec::new();
    // The receiver keeps a separate local inbox, never the user's Vault.
    let ledger_root = sources.0.join("received-inbox-ledger");
    let mut ledger = InboxLedger::open(&ledger_root).expect("open private receiver ledger");
    let mut receipts = Vec::new();
    for (slot, expected) in [&first[..], &last[..]].iter().enumerate() {
        let report = receiver.line();
        let prefix = format!("STAGED_QUEUE {slot} ");
        assert!(
            report.starts_with(&prefix),
            "unexpected Staya output: {report}"
        );
        assert!(report.contains("NOT_APPLIED=true"));
        let stage_path = report
            .strip_prefix(&prefix)
            .unwrap()
            .split(" ROOT=")
            .next()
            .expect("Staya physical stage path");
        let stage = PathBuf::from(stage_path);
        let parent = stage.parent().expect("stage parent");
        assert!(parent.starts_with(std::env::temp_dir()));
        assert!(
            parent
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("staya-direct-preview-batch-receiver-")
        );
        let actual = fs::read(&stage).expect("independent physical readback");
        assert_eq!(
            actual.as_slice(),
            *expected,
            "verified bytes must match source"
        );
        let manifest = prepared.queue.entries()[slot].manifest;
        assert_eq!(actual.len() as u64, manifest.size);
        assert_eq!(*blake3::hash(&actual).as_bytes(), manifest.id.0);

        // Feed the *same real Staya/QUIC/Bao staged bytes* to the persistent
        // receiver journal through the trusted Akula no-replace file writer.
        // The UUID is bound to the already-approved offer and selection.
        let expected_receipt = ledger_item(&offer, &prepared, slot, sender, receiver_peer);
        let mut opened_stage = fs::File::open(&stage).expect("pin the QUIC verified stage");
        let status = ledger
            .receive_with_writer(&expected_receipt, |destination| {
                aquilum_core::p2p_inbox_lab::save_verified_stage_as_new(
                    &mut opened_stage,
                    expected_receipt.bytes,
                    expected_receipt.blake3,
                    destination,
                )
                .map(|_| ())
                .map_err(|error| match error {
                    aquilum_core::p2p_inbox_lab::InboxError::Io(err) => LedgerError::Io(err),
                    aquilum_core::p2p_inbox_lab::InboxError::AlreadyExists => {
                        LedgerError::Io(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
                    }
                    _ => LedgerError::InvalidLocalObject,
                })
            })
            .expect("real transferred file must be verified into local inbox");
        assert_eq!(status, LocalStatus::VerifiedLocal);
        assert_eq!(
            fs::read(ledger.object_path(expected_receipt.id)).unwrap().as_slice(),
            *expected,
            "the persisted inbox object differs from real QUIC/Bao bytes"
        );
        assert_eq!(
            ledger
                .receive_with_writer(&expected_receipt, |_| {
                    panic!("replayed real QUIC receipt cannot create a second file")
                })
                .unwrap(),
            LocalStatus::VerifiedLocal
        );
        receipts.push(expected_receipt);

        // This is the actual bytes delivered over QUIC into Staya's private
        // stage: ask Akula Core to independently hash them while publishing
        // exactly one NEW user-chosen file. No File Gate / Vault apply.
        let inbox_target = sources.0.join(format!("accepted-inbox-{slot:02}.bin"));
        let mut pinned_stage = fs::File::open(&stage).expect("open Staya's real staged file");
        let saved = aquilum_core::p2p_inbox_lab::save_verified_stage_as_new(
            &mut pinned_stage,
            manifest.size,
            manifest.id.0,
            &inbox_target,
        )
        .expect("explicit create-new after cold stage authentication");
        assert_eq!(saved.bytes, manifest.size);
        assert_eq!(saved.blake3_root, manifest.id.0);
        let accepted_bytes = fs::read(&inbox_target).expect("read real created inbox file");
        assert_eq!(accepted_bytes.as_slice(), *expected);

        // Reusing the valid stage is not permission to replace user bytes.
        assert!(matches!(
            aquilum_core::p2p_inbox_lab::save_verified_stage_as_new(
                &mut pinned_stage,
                manifest.size,
                manifest.id.0,
                &inbox_target
            ),
            Err(aquilum_core::p2p_inbox_lab::InboxError::AlreadyExists),
        ));
        assert_eq!(fs::read(&inbox_target).unwrap().as_slice(), *expected);
        staged_directories.push(parent.to_path_buf());
    }
    assert_eq!(receiver.line(), "STY_G1K_UNLISTED_GET_DENIED=true");
    let finish = receiver.line();
    assert!(
        finish.starts_with("QUEUE_COMPLETE COUNT=2 COMMITMENT="),
        "{finish}"
    );
    assert!(finish.contains(&hex(&prepared.queue.commitment())));
    assert!(finish.contains("VERIFIED_ITEMS=2/2"));
    assert!(finish.contains("NOT_APPLIED=true"));
    assert!(receiver.finish().success(), "recipient process failed");
    drop(provider.child.stdin.take());
    assert!(provider.finish().success(), "provider process failed");
    // A cold Akula receiver process can reopen the same journal without
    // Staya, cloud metadata, sender presence or a second write.
    assert_eq!(ledger.count().unwrap(), 2);
    drop(ledger);
    let cold = InboxLedger::open(&ledger_root).expect("cold restart after QUIC transfer");
    assert_eq!(cold.count().unwrap(), 2);
    let listed = cold.list().expect("read persisted received items");
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .all(|(_, state)| *state == LocalStatus::VerifiedLocal)
    );
    for receipt in &receipts {
        assert_eq!(cold.status(receipt.id).unwrap(), LocalStatus::VerifiedLocal);
    }
    for directory in staged_directories {
        fs::remove_dir_all(directory).expect("clean only process-generated private stages");
    }
    println!(
        "AKULA_STAYA_REAL_QUIC=true two_os_processes=true selected=2 rejected=1 \
         prearm_denied=true provider_armed=true outsider_denied=true \
         real_queue=true bao_cold_readback=true inbox_cold_restart=true inbox_count=2 file_gate=false"
    );
}
