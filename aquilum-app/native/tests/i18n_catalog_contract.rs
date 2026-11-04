//! DESK-12 / L10N-01C: fail CI before a published RU/EN key or parameter drifts.
//! Pure catalog tests: no global locale mutation, GUI state, files, or OS APIs.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn flatten(value: &Value, parent: &str, entries: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(object) => {
            assert!(!object.is_empty(), "empty locale section: {parent}");
            for (key, child) in object {
                assert!(!key.is_empty() && !key.contains('.'), "invalid locale key: {parent}.{key}");
                let path = if parent.is_empty() { key.clone() } else { format!("{parent}.{key}") };
                flatten(child, &path, entries);
            }
        }
        Value::String(text) => {
            assert!(!parent.is_empty(), "locale root must be an object");
            assert!(entries.insert(parent.to_owned(), text.clone()).is_none(), "duplicate locale key: {parent}");
        }
        _ => panic!("locale leaf must be text: {parent}"),
    }
}

fn catalog(locale: &str) -> BTreeMap<String, String> {
    let raw = match locale {
        "en" => include_str!("../assets/locales/en.json"),
        "ru" => include_str!("../assets/locales/ru.json"),
        _ => panic!("unsupported locale {locale}"),
    };
    let value: Value = serde_json::from_str(raw).expect("locale JSON");
    let mut entries = BTreeMap::new();
    flatten(&value, "", &mut entries);
    entries
}

fn placeholders(key: &str, value: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = value;
    while let Some(open) = rest.find("{{") {
        assert!(!rest[..open].contains("}}"), "unexpected closing placeholder at {key}");
        rest = &rest[open + 2..];
        let end = rest.find("}}").unwrap_or_else(|| panic!("unclosed placeholder at {key}"));
        let name = &rest[..end];
        assert!(!name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "malformed placeholder at {key}: {name}");
        out.insert(name.to_owned());
        rest = &rest[end + 2..];
    }
    assert!(!rest.contains("}}"), "unexpected closing placeholder at {key}");
    out
}

fn plural_roots(en: &BTreeMap<String, String>) -> BTreeSet<String> {
    en.keys().filter_map(|key| key.strip_suffix(".one").map(ToOwned::to_owned)).collect()
}

#[test]
fn supported_locales_have_complete_keys_and_only_declared_cldr_expansions() {
    let en = catalog("en");
    let ru = catalog("ru");
    assert!(!en.is_empty() && !ru.is_empty(), "locale catalogs are empty");

    for key in en.keys() {
        assert!(ru.contains_key(key), "missing RU translation: {key}");
    }
    let roots = plural_roots(&en);
    for root in &roots {
        for suffix in ["one", "other"] {
            assert!(en.contains_key(&format!("{root}.{suffix}")), "incomplete EN plural: {root}");
        }
        for suffix in ["one", "few", "many", "other"] {
            assert!(ru.contains_key(&format!("{root}.{suffix}")), "incomplete RU plural: {root}.{suffix}");
        }
    }
    for key in ru.keys().filter(|key| !en.contains_key(*key)) {
        let Some((root, suffix)) = key.rsplit_once('.') else { panic!("unexpected RU-only key: {key}") };
        assert!(roots.contains(root) && matches!(suffix, "few" | "many"), "unreviewed RU-only key: {key}");
    }

    // These are intentionally empty because the server status panel owns the dynamic hint.
    let deliberate_empty = ["settings.server.stateHint", "settings.server.verifyHint"];
    for (locale, dict) in [("en", &en), ("ru", &ru)] {
        for (key, value) in dict {
            assert!(!value.trim().is_empty() || deliberate_empty.contains(&key.as_str()),
                "accidental empty translation in {locale}: {key}");
        }
    }
}

#[test]
fn interpolations_and_all_declared_plural_forms_match_across_languages() {
    let en = catalog("en");
    let ru = catalog("ru");
    for (key, english) in &en {
        assert_eq!(placeholders(key, english), placeholders(key, &ru[key]),
            "RU and EN placeholders drifted at {key}");
    }
    for root in plural_roots(&en) {
        let expected = placeholders(&root, &en[&format!("{root}.one")]);
        for suffix in ["one", "other"] {
            assert_eq!(placeholders(&root, &en[&format!("{root}.{suffix}")]), expected,
                "inconsistent EN plural args: {root}.{suffix}");
        }
        for suffix in ["one", "few", "many", "other"] {
            assert_eq!(placeholders(&root, &ru[&format!("{root}.{suffix}")]), expected,
                "inconsistent RU plural args: {root}.{suffix}");
        }
    }
}

#[test]
fn deletion_confirmation_is_localized_without_changing_user_file_names() {
    let en = catalog("en");
    let ru = catalog("ru");
    for (lang, dict, expected_title, expected_cancel, expected_delete) in [
        ("en", &en, "Delete the note?", "Cancel", "Delete"),
        ("ru", &ru, "Удалить заметку?", "Отмена", "Удалить"),
    ] {
        assert_eq!(dict["confirm.noteTitle"], expected_title, "{lang} title");
        assert_eq!(dict["common.cancel"], expected_cancel, "{lang} cancel");
        assert_eq!(dict["common.delete"], expected_delete, "{lang} delete");
        let name = "東京 / Пример \"<>.txt";
        let rendered = dict["confirm.noteDescription"].replace("{{name}}", name);
        assert!(rendered.contains(name), "{lang} must preserve the exact user-supplied filename");
        assert!(!rendered.contains("{{"), "{lang} has unexpanded placeholders");
        assert!(dict["confirm.deleteMany"].contains("{{count}}"), "{lang} plural confirm loses count");
    }
}
