use super::{Command, Speech, t};

pub(super) fn media(action: Speech) -> Option<Command> {
    let picker = rfd::FileDialog::new();
    if action == Speech::PickBatch {
        picker
            .set_title(t("settings.system.speech.chooseBatch"))
            .pick_files()
            .map(Command::Batch)
    } else {
        picker
            .set_title(t("settings.system.speech.mediaPicker"))
            .pick_file()
            .map(Command::Inspect)
    }
}
