use super::*;

use crate::ui::widgets::Pressed;

#[test]
fn transfer_dialog_is_reachable_and_local_queue_actions_are_real() {
    let mut h = Harness::new();
    let shortcut = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(shortcut, Pressed);
    assert!(matches!(h.app.popup, Some(Popup::Transfers(_))));
    assert!(h.app.transfer.drafts.is_empty());

    let path = h.write_note("send-only-a-draft.bin", "synthetic bytes");
    h.app.transfer_tx.send(vec![path.clone()]).unwrap();
    h.step();
    assert_eq!(h.app.transfer.drafts.len(), 1);
    assert_eq!(h.app.transfer.drafts[0].path, path);
    assert_eq!(h.app.transfer.drafts[0].bytes, 15);

    let (toggle, close) = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => (dialog.actions[0].0, dialog.close),
        _ => panic!("transfer dialog not shown after file pick"),
    };
    h.act(toggle, Pressed);
    assert!(h.app.transfer.drafts[0].paused);
    let remove = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => dialog.actions[1].0,
        _ => panic!("transfer dialog not rerendered"),
    };
    assert_ne!(remove, close);
    h.act(remove, Pressed);
    assert!(h.app.transfer.drafts.is_empty());
}

#[test]
fn closing_transfer_dialog_does_not_claim_transfer_or_write_file() {
    let mut h = Harness::new();
    let path = h.write_note("untouched.bin", "still local");
    h.app.transfer.stage(vec![path.clone()]);
    let button = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(button, Pressed);
    let close = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => dialog.close,
        _ => panic!("transfer UI did not open"),
    };
    h.act(close, Pressed);
    assert!(h.app.popup.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), b"still local");
    assert_eq!(h.app.transfer.drafts.len(), 1);
}

#[test]
fn transfer_rail_and_queue_buttons_work_through_real_pointer_events() {
    let mut h = Harness::new();
    let path = h.write_note("pointer-events.bin", "no network transfer");
    h.app.transfer.stage(vec![path.clone()]);
    let press = |h: &mut Harness, id| {
        let position = h.root.get_widget(id).expect("button exists").ctx().bounding_box().center();
        h.click(position.x, position.y);
    };
    let rail_button = h.app.chrome.as_ref().unwrap().transfer_button;
    press(&mut h, rail_button);
    let toggle = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => dialog.actions[0].0,
        _ => panic!("transfer panel did not open via a real pointer click"),
    };
    press(&mut h, toggle);
    assert!(h.app.transfer.drafts[0].paused, "pause button did not change the local draft");

    let remove = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => dialog.actions[1].0,
        _ => panic!("panel did not refresh after pause"),
    };
    press(&mut h, remove);
    assert!(h.app.transfer.drafts.is_empty(), "remove button did not remove the draft");
    // After a rerender the old close button is gone, so retrieve its new ID.
    let close = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => dialog.close,
        _ => panic!("panel did not refresh after remove"),
    };
    assert_ne!(close, remove);
    press(&mut h, close);
    assert!(h.app.popup.is_none(), "close button did not dismiss the transfer panel");
    assert_eq!(std::fs::read(&path).unwrap(), b"no network transfer", "source file was touched");
}

#[test]
fn only_identity_checked_device_can_be_selected_and_revocation_clears_selection() {
    use aquilum_core::sync::DeviceId;
    use crate::app::transfer::TransferPeer;
    use crate::ui::transfer::DeviceStatus;

    let mut h = Harness::new();
    let unknown = DeviceId::new();
    let verified = DeviceId::new();
    let revoked = DeviceId::new();
    h.app.transfer.apply_test_peer_snapshot(vec![
        TransferPeer { id: unknown, fingerprint: [1; 32], name: "Found nearby".into(), status: DeviceStatus::Unverified },
        TransferPeer { id: verified, fingerprint: [2; 32], name: "Confirmed key".into(), status: DeviceStatus::IdentityChecked },
        TransferPeer { id: revoked, fingerprint: [3; 32], name: "Former device".into(), status: DeviceStatus::Revoked },
    ]);
    let rail = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(rail, Pressed);
    let choose = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => {
            assert_eq!(dialog.device_actions.len(), 1, "unknown/revoked devices gained selection");
            dialog.device_actions[0].0
        }
        _ => panic!("device roster dialog not visible"),
    };
    // A real pointer event selects only the identified key, never starts QUIC.
    let center = h.root.get_widget(choose).unwrap().ctx().bounding_box().center();
    h.click(center.x, center.y);
    assert_eq!(h.app.transfer.selected_peer, Some((verified, [2; 32])));
    h.app.transfer.apply_test_peer_snapshot(vec![
        TransferPeer { id: verified, fingerprint: [2; 32], name: "Revoked after selection".into(), status: DeviceStatus::Revoked },
    ]);
    assert_eq!(h.app.transfer.selected_peer, None, "revoked key remained selected");
    assert!(h.app.transfer.drafts.is_empty());
}

#[test]
fn identity_rotation_and_duplicate_binding_fail_closed() {
    use aquilum_core::sync::DeviceId;
    use crate::app::transfer::TransferPeer;
    use crate::ui::transfer::DeviceStatus;

    let id = DeviceId::new();
    let mut state = crate::app::transfer::TransferState::default();
    let original = TransferPeer { id, fingerprint: [1; 32], name: "device".into(), status: DeviceStatus::IdentityChecked };
    state.apply_test_peer_snapshot(vec![original.clone()]);
    state.choose_peer((id, [1; 32]));
    assert_eq!(state.selected_peer, Some((id, [1; 32])));
    state.apply_test_peer_snapshot(vec![TransferPeer { fingerprint: [2; 32], ..original.clone() }]);
    assert_eq!(state.selected_peer, None, "key rotation retained old choice");

    state.apply_test_peer_snapshot(vec![
        original.clone(),
        TransferPeer { fingerprint: [3; 32], ..original.clone() },
    ]);
    assert!(state.device_rows().iter().all(|row| row.status == DeviceStatus::Unverified));
    state.choose_peer((id, [1; 32]));
    assert_eq!(state.selected_peer, None, "ambiguous identity selected");

    state.apply_test_peer_snapshot(vec![TransferPeer { fingerprint: [0; 32], ..original }]);
    assert_eq!(state.device_rows()[0].status, DeviceStatus::Unverified);
    state.choose_peer((id, [0; 32]));
    assert_eq!(state.selected_peer, None, "zero fingerprint selected");
}
#[test]
fn stale_device_button_cannot_select_a_reordered_peer() {
    use aquilum_core::sync::DeviceId;
    use crate::app::transfer::TransferPeer;
    use crate::ui::transfer::DeviceStatus;

    let mut h = Harness::new();
    let old = DeviceId::new();
    let other = DeviceId::new();
    h.app.transfer.apply_test_peer_snapshot(vec![
        TransferPeer { id: old, fingerprint: [7; 32], name: "old".into(), status: DeviceStatus::IdentityChecked },
    ]);
    let rail = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(rail, Pressed);
    let stale = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => dialog.device_actions[0].0,
        _ => panic!("roster not shown"),
    };
    // Mutate test-only roster without refreshing the visible modal: the old
    // button must not select the *replacement* identity at the same index.
    h.app.transfer.apply_test_peer_snapshot(vec![
        TransferPeer { id: other, fingerprint: [8; 32], name: "new".into(), status: DeviceStatus::IdentityChecked },
    ]);
    h.act(stale, Pressed);
    assert_eq!(h.app.transfer.selected_peer, None, "stale click selected another DeviceId");
    assert!(h.app.transfer.drafts.is_empty());
}

#[test]
fn identical_duplicate_device_ids_and_oversize_rosters_are_unselectable() {
    use aquilum_core::sync::DeviceId;
    use crate::app::transfer::{TransferPeer, TransferState};
    use crate::ui::transfer::DeviceStatus;
    let id = DeviceId::new();
    let row = TransferPeer { id, fingerprint: [3; 32], name: "same".into(), status: DeviceStatus::IdentityChecked };
    let mut state = TransferState::default();
    state.apply_test_peer_snapshot(vec![row.clone(), row.clone()]);
    assert!(state.device_rows().iter().all(|entry| entry.status == DeviceStatus::Unverified));
    state.choose_peer((id, [3; 32]));
    assert_eq!(state.selected_peer, None);
    state.apply_test_peer_snapshot(vec![row; 33]);
    assert!(state.device_rows().is_empty());
    assert_eq!(state.selected_peer, None);
}
