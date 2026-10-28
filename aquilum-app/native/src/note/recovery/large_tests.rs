//! 100k-line, >16 MiB Unicode shadow survives abnormal exit before debounce.
use std::{fmt::Write as _, fs, path::PathBuf, process::Command, time::Duration};
use crop::Rope;
use uuid::Uuid;
use super::{MAX_TEXT, inspect};

fn corpus() -> String {
    let mut text = String::with_capacity(20_000_000);
    for i in 0..100_000 {
        if i % 80 == 0 {
            writeln!(&mut text, "## Раздел {i}: работа с большой заметкой").unwrap();
        } else {
            writeln!(
                &mut text,
                "Строка {i:06}: русский текст и UTF-8, **жирный**, *курсив*, [[Akula]]; \
                 длинная фраза для переноса текста в окне редактора."
            ).unwrap();
        }
    }
    assert!(text.len() > 16 * 1024 * 1024 && text.len() <= MAX_TEXT);
    text
}

#[test]
fn large_dirty_note_child_exits_before_file_gate_debounce() {
    let Ok(data) = std::env::var("AKULA_LARGE_RECOVERY_CHILD_DATA") else { return };
    let vault = PathBuf::from(std::env::var("AKULA_LARGE_RECOVERY_CHILD_VAULT").unwrap());
    let workspace = Uuid::parse_str(&std::env::var("AKULA_LARGE_RECOVERY_CHILD_WORKSPACE").unwrap()).unwrap();
    let path = vault.join("large.md");
    let (mut note, _) = crate::note::Note::open(path).unwrap();
    note.arm_shadow(std::path::Path::new(&data), &vault, workspace).unwrap();
    let text = Rope::from(corpus());
    note.edit(Duration::from_secs(30));
    let revision = note.snapshot_unsaved(&text).unwrap().unwrap();
    crate::disk::wait();
    assert_eq!(note.shadow_status().unwrap().durable_revision, Some(revision));
    assert!(note.shadow_status().unwrap().error.is_none());
    std::process::exit(49);
}

#[test]
fn hundred_thousand_lines_over_16m_survive_abnormal_exit() {
    let data = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let path = vault.path().join("large.md");
    fs::write(&path, "last saved baseline").unwrap();
    let workspace = Uuid::new_v4();
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("large_dirty_note_child_exits_before_file_gate_debounce")
        .arg("--nocapture")
        .env("AKULA_LARGE_RECOVERY_CHILD_DATA", data.path())
        .env("AKULA_LARGE_RECOVERY_CHILD_VAULT", vault.path())
        .env("AKULA_LARGE_RECOVERY_CHILD_WORKSPACE", workspace.to_string())
        .status().unwrap();
    assert_eq!(child.code(), Some(49));
    assert_eq!(fs::read_to_string(&path).unwrap(), "last saved baseline");
    let recovered = inspect(data.path(), vault.path(), workspace, &path).unwrap().unwrap();
    let expected = corpus();
    assert_eq!(recovered.body.len(), expected.len());
    assert_eq!(blake3::hash(recovered.body.as_bytes()), blake3::hash(expected.as_bytes()));
    assert_eq!(recovered.body, expected);
    assert!(!recovered.already_saved && !recovered.source_changed);
}
