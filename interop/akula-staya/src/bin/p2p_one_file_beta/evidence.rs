//! Explicitly opted-in B2 evidence from two real Windows beta processes.
//! No filesystem paths, peer IDs, addresses, private keys or relay URLs.
//! A snapshot at one instant is not proof of the path used for every byte.
//! Output hashes can be sensitive: off by default, use synthetic test files.
use iroh::endpoint::Connection;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(super) enum Phase {
    SenderAck,
    ReceiverInbox,
}

fn selected_path_snapshot(connection: &Connection) -> &'static str {
    let paths = connection.paths();
    match paths.iter().find(|path| path.is_selected()) {
        Some(path) if path.is_ip() => "direct",
        Some(path) if path.is_relay() => "relay",
        _ => "unknown",
    }
}

fn line(
    phase: Phase,
    bytes: u64,
    digest: &[u8; 32],
    receipt: Option<Uuid>,
    path_snapshot: &str,
) -> String {
    let phase = match phase {
        Phase::SenderAck => "sender_verified_peer_ack",
        Phase::ReceiverInbox => "receiver_local_inbox_verified",
    };
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let receipt = receipt.map_or_else(|| "none".to_owned(), |id| id.to_string());
    format!(
        "AKULA_B2_QA_EVIDENCE phase={phase} bytes={bytes} blake3={hash}          inbox_receipt={receipt} selected_quic_path_snapshot={path_snapshot}          durable_ack=false signed_offer=false"
    )
}

pub(super) fn emit_if_opted_in(
    connection: &Connection,
    phase: Phase,
    bytes: u64,
    digest: &[u8; 32],
    receipt: Option<Uuid>,
) {
    // Disabled by default to avoid logging hashes of real user content.
    if std::env::var("AKULA_P2P_B2_QA_EVIDENCE").as_deref() != Ok("1") {
        return;
    }
    println!(
        "{}",
        line(phase, bytes, digest, receipt, selected_path_snapshot(connection))
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_and_receiver_proofs_correlate_but_never_claim_durable_delivery() {
        let bytes = b"AKULA synthetic WAN B2 test fixture only";
        let digest = *blake3::hash(bytes).as_bytes();
        let receipt = Uuid::parse_str("05ec121e-0f1b-4a63-8c4a-5e11c32a1b41").unwrap();
        let send = line(Phase::SenderAck, bytes.len() as u64, &digest, None, "direct");
        let receive = line(
            Phase::ReceiverInbox,
            bytes.len() as u64,
            &digest,
            Some(receipt),
            "relay",
        );
        let shared = format!(
            "bytes={} blake3={}",
            bytes.len(),
            digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        );
        assert!(send.contains(&shared));
        assert!(receive.contains(&shared));
        assert!(receive.contains(&format!("inbox_receipt={receipt}")));
        assert!(send.contains("selected_quic_path_snapshot=direct"));
        assert!(receive.contains("selected_quic_path_snapshot=relay"));
        for proof in [send, receive] {
            assert!(proof.contains("durable_ack=false"));
            assert!(proof.contains("signed_offer=false"));
            assert!(!proof.contains("user\\"));
            assert!(!proof.contains("192.168."));
            assert!(!proof.contains("relay.example"));
        }
    }

    #[test]
    fn no_receipt_or_network_route_is_invented() {
        let digest = *blake3::hash(b"test").as_bytes();
        let proof = line(Phase::SenderAck, 4, &digest, None, "unknown");
        assert!(proof.contains("inbox_receipt=none"));
        assert!(proof.contains("selected_quic_path_snapshot=unknown"));
        assert!(!proof.contains("durable_ack=true"));
    }
}
