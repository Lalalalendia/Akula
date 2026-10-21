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
