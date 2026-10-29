//! QA-only, read-only Vault projection of locally reverified P2P imports.
//! This intentionally does not admit arbitrary .bin files into the note tree.
use std::path::Path;

use akula_inbox_ledger_lab::{InboxLedger, user_inbox_root};
use masonry::core::{NewWidget, PropertySet};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use crate::i18n::t;
use crate::ui::tokens::size;
use crate::ui::widgets::Label;

fn verified_names(vault: &Path, inbox: &Path) -> Vec<String> {
    let Ok(Some(ledger)) = InboxLedger::open_existing(inbox) else {
        return Vec::new();
    };
    ledger
        .verified_imports_in(vault)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect()
}

pub(super) fn render(vault: &Path) -> Option<NewWidget<Flex>> {
    let inbox = user_inbox_root().ok()?;
    let _pinned = aquilum_core::p2p_inbox_lab::pin_windows_inbox_read(&inbox).ok()?;
    let names = verified_names(vault, &inbox);
    if names.is_empty() {
        return None;
    }
    let mut panel = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(NewWidget::new(Label::new(
            &t("transfer.inboxVaultCopies"),
            |t| t.text_secondary,
        )));
    for name in names {
        panel = panel.with_fixed(NewWidget::new(Label::new(&name, |t| t.text_secondary)));
    }
    panel = panel.with_fixed(NewWidget::new(Label::new(
        &t("transfer.inboxVaultCopiesHint"),
        |t| t.text_tertiary,
    )));
    Some(NewWidget::new(panel).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::SPACE_8))),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_inbox_ledger_lab::{ExpectedTransfer, Transport};
    use std::{fs, io::Write};
    use uuid::Uuid;

    #[test]
    fn only_recorded_reverified_vault_copy_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("inbox");
        let vault = dir.path().join("vault");
        let other = dir.path().join("other");
        fs::create_dir(&vault).unwrap();
        fs::create_dir(&other).unwrap();
        let bytes = b"checked local p2p import";
        let intent = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [11; 32],
            recipient_key: [12; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let mut ledger = InboxLedger::open(&inbox).unwrap();
        ledger.reserve(&intent).unwrap();
        let original = ledger.object_path(intent.id);
        let mut writer = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&original)
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        ledger.confirm(&intent).unwrap();

        let name = format!("received-{}.bin", intent.id);
        let target = vault.join(&name);
        fs::write(&target, bytes).unwrap();
        assert!(verified_names(&vault, &inbox).is_empty());
        ledger.record_imported(intent.id, &target).unwrap();
        assert_eq!(verified_names(&vault, &inbox), vec![name]);
        assert!(verified_names(&other, &inbox).is_empty());
        fs::write(&target, b"modified after import").unwrap();
        assert!(verified_names(&vault, &inbox).is_empty());
        assert_eq!(fs::read(&original).unwrap(), bytes);
    }
}
