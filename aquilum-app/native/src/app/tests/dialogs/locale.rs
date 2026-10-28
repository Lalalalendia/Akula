use super::*;
use crate::i18n;
use crate::ui::settings::schema::{Setter, Value};

fn select_language(config: &mut AppConfig, value: &Value) {
    if let Value::Text(language) = value {
        config.ui.language = language.clone();
    }
}

fn change_language(harness: &mut Harness, language: &str) {
    let effect = harness.app.apply(
        Setter::Config(select_language),
        Value::Text(language.to_owned()),
    );
    assert!(matches!(effect, crate::app::settings::Effect::Everything),
        "switching the UI language must rebuild the native widget tree");
    harness.app.rebuild(&mut harness.root);
    harness.settle();
    assert_eq!(harness.config().ui.language, language);
}

#[test]
fn ru_en_switch_rebuilds_settings_and_survives_restart_without_mutating_note() {
    let mut h = Harness::new();
    let note = h.write_note("日本語 Пример.md", "Исходный текст — 東京 / 中文 / 한국어");
    let original_bytes = std::fs::read(&note).unwrap();
    h.app.open(&mut h.root, note.clone());
    h.settle();
    assert_eq!(h.config().ui.language, "ru");
    assert_eq!(i18n::t("confirm.noteTitle"), "Удалить заметку?");
    assert_eq!(h.body_text().as_bytes(), original_bytes);

    h.app.open_settings(&mut h.root);
    assert!(h.section().is_some(), "native settings must be open before switching languages");
    change_language(&mut h, "en");
    assert!(h.section().is_some(), "rebuild must preserve the settings dialog");
    assert_eq!(i18n::t("confirm.noteTitle"), "Delete the note?");
    assert_eq!(i18n::t("common.cancel"), "Cancel");
    assert_eq!(h.body_text().as_bytes(), original_bytes);
    assert_eq!(std::fs::read(&note).unwrap(), original_bytes, "language must not rewrite note");

    h.escape();
    let mut h = h.restarted(1.0);
    assert_eq!(h.config().ui.language, "en", "selected locale must survive restart");
    assert_eq!(i18n::t("confirm.noteTitle"), "Delete the note?");
    h.app.open(&mut h.root, note.clone());
    h.settle();
    assert_eq!(h.body_text().as_bytes(), original_bytes);
    assert_eq!(std::fs::read(&note).unwrap(), original_bytes);

    h.app.open_settings(&mut h.root);
    change_language(&mut h, "ru");
    assert_eq!(i18n::t("confirm.noteTitle"), "Удалить заметку?");
    assert_eq!(i18n::t("common.cancel"), "Отмена");
    assert_eq!(h.body_text().as_bytes(), original_bytes);
    assert_eq!(std::fs::read(&note).unwrap(), original_bytes);
}
