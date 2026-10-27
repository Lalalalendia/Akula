use super::document::{
    hash_bytes, normalize_line_endings, read_file_hash_impl, read_file_snapshot_impl, read_file_stat_impl, write_file_atomic_impl,
};
use super::entries::{create_file_impl, rename_file_impl};
use super::error::FileCommandError;
use super::models::FileItemType;
use super::workspace::read_directory_impl;
use std::fs;

#[test]
fn stat_returns_byte_length_without_reading_content() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("book.epub");
    fs::write(&path, b"0123456789abcdef").expect("fixture");

    let stat = read_file_stat_impl(&path).expect("stat");

    assert_eq!(stat.byte_length, 16);
}

#[test]
fn snapshot_contains_content_and_hash() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("note.md");
    fs::write(&path, "Привет, Aquilum!").expect("fixture");

    let snapshot = read_file_snapshot_impl(&path).expect("snapshot");

    assert_eq!(snapshot.content, "Привет, Aquilum!");
    assert_eq!(snapshot.hash, hash_bytes(snapshot.content.as_bytes()));
}

#[test]
fn atomic_write_replaces_the_complete_file() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("note.md");
    fs::write(&path, "before").expect("fixture");

    let result = write_file_atomic_impl(&path, "after", None).expect("write");

    assert_eq!(fs::read_to_string(&path).expect("result"), "after");
    assert_eq!(result.hash, hash_bytes(b"after"));
}

#[test]
fn expected_hash_prevents_overwrite() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("note.md");
    fs::write(&path, "external change").expect("fixture");

    let error = write_file_atomic_impl(&path, "aquilum change", Some("stale-hash"))
        .expect_err("conflict expected");

    assert!(matches!(error, FileCommandError::Conflict { .. }));
    assert_eq!(
        fs::read_to_string(&path).expect("unchanged file"),
        "external change"
    );
}

#[test]
fn create_file_never_overwrites_an_existing_note() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("note.md");
    fs::write(&path, "keep me").expect("fixture");

    let error = create_file_impl(&path, "replacement").expect_err("already exists");

    assert!(matches!(error, FileCommandError::AlreadyExists { .. }));
    assert_eq!(fs::read_to_string(&path).expect("result"), "keep me");
}

#[test]
fn rename_file_never_overwrites_an_existing_note() {
    let directory = tempfile::tempdir().expect("temp directory");
    let old_path = directory.path().join("old.md");
    let new_path = directory.path().join("new.md");
    fs::write(&old_path, "old content").expect("old fixture");
    fs::write(&new_path, "new content").expect("new fixture");

    let error = rename_file_impl(&old_path, &new_path).expect_err("already exists");

    assert!(matches!(error, FileCommandError::AlreadyExists { .. }));
    assert_eq!(
        fs::read_to_string(old_path).expect("old result"),
        "old content"
    );
    assert_eq!(
        fs::read_to_string(new_path).expect("new result"),
        "new content"
    );
}

#[test]
fn rename_file_moves_content_to_a_new_name() {
    let directory = tempfile::tempdir().expect("temp directory");
    let old_path = directory.path().join("old.md");
    let new_path = directory.path().join("new.md");
    fs::write(&old_path, "content").expect("fixture");

    rename_file_impl(&old_path, &new_path).expect("rename");

    assert!(!old_path.exists());
    assert_eq!(fs::read_to_string(new_path).expect("result"), "content");
}

#[test]
fn invalid_utf8_is_rejected() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("note.md");
    fs::write(&path, [0xff, 0xfe]).expect("fixture");

    let error = read_file_snapshot_impl(&path).expect_err("invalid UTF-8");

    assert!(matches!(error, FileCommandError::InvalidUtf8 { .. }));
}

#[test]
fn command_error_has_a_stable_serialized_code() {
    let error = FileCommandError::AlreadyExists {
        path: "note.md".to_owned(),
    };

    let value = serde_json::to_value(error).expect("serialized error");

    assert_eq!(value["code"], "already_exists");
    assert_eq!(value["details"]["path"], "note.md");
}

#[test]
fn workspace_filters_and_sorts_entries() {
    let directory = tempfile::tempdir().expect("temp directory");
    fs::create_dir(directory.path().join("Folder")).expect("folder fixture");
    fs::write(directory.path().join("Beta.md"), "").expect("beta fixture");
    fs::write(directory.path().join("alpha.md"), "").expect("alpha fixture");
    fs::write(directory.path().join("Gamma.MD"), "").expect("gamma fixture");
    fs::write(directory.path().join("photo.png"), "").expect("image fixture");
    fs::write(directory.path().join("doc.pdf"), "").expect("pdf fixture");
    fs::write(directory.path().join("novel.epub"), "").expect("epub fixture");
    fs::write(directory.path().join("notes.txt"), "").expect("text fixture");
    fs::write(directory.path().join(".hidden.md"), "").expect("hidden fixture");

    let items = read_directory_impl(directory.path()).expect("workspace");
    let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();

    assert_eq!(
        names,
        vec![
            "Folder",
            ".hidden",
            "alpha",
            "Beta",
            "doc.pdf",
            "Gamma",
            "notes.txt",
            "novel.epub",
            "photo.png"
        ]
    );
    assert_eq!(items[0].item_type, FileItemType::Folder);
    assert_eq!(items[1].item_type, FileItemType::File);
}

#[test]
fn a_snapshot_normalises_crlf_but_keeps_the_hash_of_the_bytes_on_disk() {
    let directory = tempfile::tempdir().expect("workspace");
    let path = directory.path().join("Заметка.md");
    fs::write(&path, "первая\r\nвторая\r\nтретья").expect("fixture");

    let snapshot = read_file_snapshot_impl(&path).expect("snapshot");

    assert_eq!(snapshot.content, "первая\nвторая\nтретья");
    assert_eq!(
        snapshot.hash,
        hash_bytes("первая\r\nвторая\r\nтретья".as_bytes()),
        "хэш остаётся хэшем файла: на нём держится проверка при записи"
    );
    assert_eq!(
        snapshot.text_hash,
        hash_bytes("первая\nвторая\nтретья".as_bytes()),
        "text_hash считается от нормализованного текста — его и видит документ в ядре"
    );
}

#[test]
fn a_snapshot_of_an_lf_file_reports_the_same_hash_twice() {
    let directory = tempfile::tempdir().expect("workspace");
    let path = directory.path().join("Заметка.md");
    fs::write(&path, "первая\nвторая").expect("fixture");

    let snapshot = read_file_snapshot_impl(&path).expect("snapshot");

    assert_eq!(snapshot.content, "первая\nвторая");
    assert_eq!(snapshot.hash, snapshot.text_hash);
}

#[test]
fn a_lone_carriage_return_is_normalised_too() {
    assert_eq!(normalize_line_endings("одна\rдве".to_owned()), "одна\nдве");
    assert_eq!(normalize_line_endings("без переносов".to_owned()), "без переносов");
    assert_eq!(normalize_line_endings("\r\rа\r\nб\r\r\nв\n\r".to_owned()), "\n\nа\nб\n\nв\n\n");
}

#[test]
fn raw_hash_is_streamed_without_changing_conflict_or_crlf_semantics() {
    let directory = tempfile::tempdir().expect("workspace");
    let path = directory.path().join("большая заметка.md");
    let raw = "Unicode 😀 и строка\r\n".repeat(40_000) + "завершение\rхвост";
    fs::write(&path, &raw).expect("fixture");

    let fingerprint = read_file_hash_impl(&path).expect("streamed hash");
    assert_eq!(fingerprint, hash_bytes(raw.as_bytes()), "raw CRLF bytes must be hashed");
    assert_eq!(fingerprint, read_file_snapshot_impl(&path).unwrap().hash);

    let saved = write_file_atomic_impl(&path, "наша правка", Some(&fingerprint)).expect("fresh expected hash");
    assert_eq!(saved.hash, hash_bytes("наша правка".as_bytes()));
    fs::write(&path, "чужая правка").expect("external writer");
    let error = write_file_atomic_impl(&path, "не затирать", Some(&saved.hash)).unwrap_err();
    assert!(matches!(error, FileCommandError::Conflict { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "чужая правка");
}

#[test]
fn streamed_hash_of_empty_and_binary_files_matches_byte_hash() {
    let directory = tempfile::tempdir().expect("workspace");
    let path = directory.path().join("bytes");
    fs::write(&path, b"").expect("empty fixture");
    assert_eq!(read_file_hash_impl(&path).unwrap(), hash_bytes(b""));
    let bytes = [0, 255, 128, 13, 10, 0, 7];
    fs::write(&path, bytes).expect("binary fixture");
    assert_eq!(read_file_hash_impl(&path).unwrap(), hash_bytes(&bytes));
}


#[test]
fn structured_extensions_are_discovered_but_not_markdown_notes() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("Board.canvas"), "canvas").unwrap();
    fs::write(directory.path().join("Tasks.BASE"), "base").unwrap();
    let items = read_directory_impl(directory.path()).unwrap();
    let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
    assert!(names.contains(&"Board.canvas"));
    assert!(names.contains(&"Tasks.BASE"));
    assert!(!crate::search::paths::is_note(&directory.path().join("Board.canvas")));
    assert!(!crate::search::paths::is_note(&directory.path().join("Tasks.BASE")));
}

#[test]
fn user_dot_folders_are_listed_but_application_folders_remain_hidden() {
    let dir = tempfile::tempdir().unwrap();
    for folder in [".заметки", ".git", ".aquilum", ".trash", ".quantum", ".obsidian", ".akula-sync-stage-temp"] {
        fs::create_dir(dir.path().join(folder)).unwrap();
    }
    fs::write(dir.path().join(".hidden.md"), b"visible user note").unwrap();
    fs::write(dir.path().join("._mac.md"), b"hidden metadata").unwrap();
    let items = read_directory_impl(dir.path()).unwrap();
    let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, vec![".заметки", ".hidden"]);
}

#[test]
fn workspace_preflight_preserves_case_unicode_ids_and_supported_formats() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in [".архив", "Documents", ".GIT", ".AKULA-backup"] {
        fs::create_dir(root.join(folder)).unwrap();
    }
    for file in [
        ".draft.txt", "Book.EPUB", "Image.PNG", "Notes.MD", "Paper.PDF",
        "text.TXT", "external.bin", "unrecognized.custom", "._staging.md",
    ] {
        fs::write(root.join(file), b"file").unwrap();
    }
    let items = read_directory_impl(root).unwrap();
    let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, vec![
        ".архив", "Documents", ".draft.txt", "Book.EPUB",
        "Image.PNG", "Notes", "Paper.PDF", "text.TXT",
    ]);
    for item in &items {
        let actual = root.join(if item.name == "Notes" { "Notes.MD" } else { item.name.as_str() });
        assert_eq!(item.id, actual.to_string_lossy().as_ref(), "ID must be the unchanged filesystem path");
    }
    assert_eq!(items.iter().filter(|item| item.item_type == FileItemType::Folder).count(), 2);
}

#[test]
#[ignore = "opt-in same-runner A/B of actual directory listing, not FileTree startup FPS"]
fn benchmark_workspace_scan_abba() {
    use std::hint::black_box;
    use std::time::Instant;

    let corpus = std::path::PathBuf::from(
        std::env::var_os("AKULA_WORKSPACE_SCAN_ABBA_DIR")
            .expect("set AKULA_WORKSPACE_SCAN_ABBA_DIR to the pre-created deterministic corpus"),
    );
    for cohort in ["mostly_unsupported", "mixed", "mostly_supported"] {
        let path = corpus.join(cohort);
        let reference = read_directory_impl(&path).expect("initial directory scan");
        let mut fingerprint = 0xcbf29ce484222325u64;
        for item in &reference {
            for &byte in item.id.as_bytes().iter().chain(item.name.as_bytes()) {
                fingerprint = (fingerprint ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
            fingerprint = (fingerprint ^ u64::from(item.item_type == FileItemType::File))
                .wrapping_mul(0x100000001b3);
        }
        let expected = reference.len();
        for _ in 0..4 {
            black_box(read_directory_impl(&path).expect("warmup scan"));
        }
        let mut samples = Vec::with_capacity(40);
        for _ in 0..40 {
            let started = Instant::now();
            let items = read_directory_impl(&path).expect("timed scan");
            let elapsed = started.elapsed();
            assert_eq!(items.len(), expected, "scan changed item count");
            black_box(items);
            samples.push(elapsed.as_secs_f64() * 1_000_000.0);
        }
        samples.sort_by(f64::total_cmp);
        let p50 = (samples[19] + samples[20]) / 2.0;
        let p95 = samples[37];
        eprintln!(
            "WORKSPACE_SCAN_ABBA cohort={cohort} accepted={expected} checksum={fingerprint:016x} \
             runs=40 p50_us={p50:.2} p95_us={p95:.2}"
        );
    }
}
