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
