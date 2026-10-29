use super::*;

const NOW: u64 = 1_800_000_000;
fn peer(id: u128, key: u8) -> PeerBinding {
    PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_u128(id)),
        endpoint_id: [key; 32],
    }
}
fn offer() -> OfferV1 {
    OfferV1 {
        schema: SCHEMA,
        offer_id: Uuid::from_u128(99),
        nonce: [7; 16],
        sender: peer(1, 3),
        receiver: peer(2, 4),
        created_unix: NOW,
        expires_unix: NOW + 120,
        files: vec![
            OfferedFile {
                label: "first.txt".into(),
                bytes: 3,
                blake3_root: [1; 32],
            },
            OfferedFile {
                label: "second.bin".into(),
                bytes: 7,
                blake3_root: [2; 32],
            },
            OfferedFile {
                label: "third.txt".into(),
                bytes: 11,
                blake3_root: [3; 32],
            },
        ],
    }
}
fn decision(x: &OfferV1, selected: Vec<u8>) -> DecisionV1 {
    x.receiver_decision(
        DecisionAction::Accept,
        selected,
        NOW + 1,
        x.sender,
        x.receiver,
    )
    .unwrap()
}

#[test]
fn roundtrip_partial_accept_has_only_ui_draft() {
    let x = offer();
    let wire = x.encode(NOW).unwrap();
    let receiver = OfferV1::decode_for_receiver(&wire, NOW + 1, x.sender, x.receiver).unwrap();
    let d = decision(&receiver, vec![0, 2]);
    let d = DecisionV1::decode(&d.encode().unwrap()).unwrap();
    let mut session = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
    let Outcome::Selected(draft) = session.resolve(&d, x.receiver, NOW + 2).unwrap() else {
        panic!("not a selection");
    };
    assert_eq!(
        draft
            .files
            .iter()
            .map(|x| x.label.as_str())
            .collect::<Vec<_>>(),
        vec!["first.txt", "third.txt"]
    );
    assert_eq!(session.phase(), Phase::Accepted);
}

#[test]
fn forgery_of_transport_identity_or_application_device_is_rejected() {
    let x = offer();
    let wire = x.encode(NOW).unwrap();
    assert_eq!(
        OfferV1::decode_for_receiver(&wire, NOW, peer(44, 3), x.receiver),
        Err(OfferError::WrongIdentity)
    );
    assert_eq!(
        OfferV1::decode_for_receiver(&wire, NOW, x.sender, peer(2, 77)),
        Err(OfferError::WrongIdentity)
    );
    let mut session = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
    assert_eq!(
        session.resolve(&decision(&x, vec![0]), peer(44, 44), NOW + 1),
        Err(OfferError::WrongIdentity)
    );
    assert_eq!(session.phase(), Phase::Pending);
}

#[test]
fn changing_manifest_size_root_or_offer_id_invalidates_decision() {
    let x = offer();
    let d = decision(&x, vec![0]);
    for n in 0..3 {
        let mut altered = x.clone();
        if n == 0 {
            altered.files[0].bytes += 1;
        }
        if n == 1 {
            altered.files[0].blake3_root = [88; 32];
        }
        if n == 2 {
            altered.offer_id = Uuid::from_u128(100);
        }
        let mut s =
            OfferSession::new_sender(altered.clone(), NOW, altered.sender, altered.receiver)
                .unwrap();
        assert_eq!(
            s.resolve(&d, altered.receiver, NOW + 1),
            Err(OfferError::WrongCommitment)
        );
    }
}

#[test]
fn accept_is_single_use_and_cancellation_is_terminal() {
    let x = offer();
    let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
    let d = decision(&x, vec![0]);
    s.resolve(&d, x.receiver, NOW + 1).unwrap();
    assert_eq!(
        s.resolve(&d, x.receiver, NOW + 2),
        Err(OfferError::AlreadyResolved)
    );
    let mut other = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
    other.cancel().unwrap();
    assert_eq!(
        other.resolve(&d, x.receiver, NOW + 1),
        Err(OfferError::AlreadyResolved)
    );
}

#[test]
fn decline_is_terminal_and_carries_no_selected_files() {
    let x = offer();
    let d = x
        .receiver_decision(DecisionAction::Decline, vec![], NOW, x.sender, x.receiver)
        .unwrap();
    let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
    assert_eq!(s.resolve(&d, x.receiver, NOW + 1), Ok(Outcome::Declined));
    assert_eq!(
        s.resolve(&decision(&x, vec![0]), x.receiver, NOW + 2),
        Err(OfferError::AlreadyResolved)
    );
}

#[test]
fn nil_application_device_identity_is_rejected() {
    let mut bad = offer();
    bad.sender.device_id = DeviceId::from_uuid(Uuid::nil());
    assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));

    bad = offer();
    bad.receiver.device_id = DeviceId::from_uuid(Uuid::nil());
    assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));
}

#[test]
fn expiry_and_future_time_fail_closed() {
    let x = offer();
    assert_eq!(x.validate(NOW - 1), Err(OfferError::NotYetValid));
    assert_eq!(x.validate(NOW + 120), Err(OfferError::Expired));
    let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
    assert_eq!(
        s.resolve(&decision(&x, vec![0]), x.receiver, NOW + 120),
        Err(OfferError::Expired)
    );
    assert_eq!(s.phase(), Phase::Expired);
}

#[test]
fn invalid_selection_is_rejected() {
    let x = offer();
    for bad in [vec![], vec![1, 1], vec![2, 0], vec![3], vec![0, 1, 2, 3]] {
        assert_eq!(
            x.receiver_decision(DecisionAction::Accept, bad, NOW, x.sender, x.receiver),
            Err(OfferError::InvalidSelection)
        );
    }
    assert_eq!(
        x.receiver_decision(DecisionAction::Decline, vec![0], NOW, x.sender, x.receiver),
        Err(OfferError::InvalidSelection)
    );
}

#[test]
fn reject_quota_duplicate_and_dangerous_labels() {
    let x = offer();
    let mut bad = x.clone();
    bad.files[0].bytes = MAX_ITEM_BYTES + 1;
    assert_eq!(bad.validate(NOW), Err(OfferError::TooLarge));
    bad = x.clone();
    bad.files[1].label = "FIRST.TXT".into();
    assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));
    bad = x.clone();
    bad.files[1].blake3_root = [1; 32];
    assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));
    for label in [
        "../x",
        "folder\\x",
        "CON",
        "LPT9.txt",
        "file.",
        "a\u{202e}txt",
    ] {
        bad = x.clone();
        bad.files[0].label = label.into();
        assert_eq!(
            bad.validate(NOW),
            Err(OfferError::InvalidMetadata),
            "{label}"
        );
    }
    bad = x;
    bad.files = (0u8..9)
        .map(|i| OfferedFile {
            label: format!("file{i}"),
            bytes: 1,
            blake3_root: [i + 1; 32],
        })
        .collect();
    assert_eq!(bad.validate(NOW), Err(OfferError::TooLarge));
}

#[test]
fn unknown_fields_bad_schema_and_oversized_frame_refused() {
    let x = offer();
    let mut doc = serde_json::to_value(&x).unwrap();
    doc["files"][0]["writeVault"] = serde_json::json!(true);
    assert_eq!(
        OfferV1::decode_for_receiver(
            &serde_json::to_vec(&doc).unwrap(),
            NOW,
            x.sender,
            x.receiver
        ),
        Err(OfferError::InvalidEncoding)
    );
    let mut x2 = x.clone();
    x2.schema = 9;
    assert_eq!(x2.validate(NOW), Err(OfferError::UnsupportedSchema));
    assert_eq!(
        OfferV1::decode_for_receiver(
            &vec![b' '; MAX_OFFER_WIRE + 1],
            NOW,
            x.sender,
            x.receiver
        ),
        Err(OfferError::TooLarge)
    );
    assert_eq!(
        DecisionV1::decode(&vec![b' '; MAX_DECISION_WIRE + 1]),
        Err(OfferError::TooLarge)
    );
}

#[test]
fn wrong_decision_binding_and_unknown_authority_field_fail() {
    let x = offer();
    let mut d = decision(&x, vec![0]);
    let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
    d.manifest_commitment = [99; 32];
    assert_eq!(
        s.resolve(&d, x.receiver, NOW + 1),
        Err(OfferError::WrongCommitment)
    );
    let mut json = serde_json::to_value(&d).unwrap();
    json["grant"] = serde_json::json!(true);
    assert_eq!(
        DecisionV1::decode(&serde_json::to_vec(&json).unwrap()),
        Err(OfferError::InvalidEncoding)
    );
    assert_eq!(s.phase(), Phase::Pending);
}
