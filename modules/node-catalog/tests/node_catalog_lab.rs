//! D1.5: source-level read catalog only, intentionally NO HTTP server.
//! All document names are operator allowlisted and bound to pinned Linux fds.

#![cfg(target_os = "linux")]

use akula_node_catalog::{CatalogError, ReadCatalog, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS};
use akula_node_catalog::VaultId;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn vault_id() -> VaultId {
    VaultId::from_uuid(uuid::Uuid::from_u128(1))
}

fn setup() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    fs::create_dir(&root).unwrap();
    (tmp, root)
}

fn id_of(catalog: &ReadCatalog, name: &str) -> String {
    catalog.list().into_iter().find(|item| item.filename == name)
        .expect("allowlisted object id").id
}

fn invalid(root: &Path, files: &[&str]) {
    assert_eq!(ReadCatalog::open(root, vault_id(), files).err(), Some(CatalogError::InvalidConfiguration));
}

#[test]
fn three_allowlisted_documents_have_stable_ids_and_exact_raw_revisions() {
    let (_scratch, root) = setup();
    let docs = [
        ("alpha.md", b"# Alpha\r\n".as_slice()),
        ("beta.md", b"# Beta\n".as_slice()),
        ("gamma.md", "Привет, мир!\n".as_bytes()),
    ];
    for (name, bytes) in docs { fs::write(root.join(name), bytes).unwrap(); }
    fs::write(root.join("private.md"), b"DENIED").unwrap();

    let first = ReadCatalog::open(&root, vault_id(), &["alpha.md", "beta.md", "gamma.md"]).unwrap();
    let second = ReadCatalog::open(&root, vault_id(), &["gamma.md", "alpha.md", "beta.md"]).unwrap();
    assert_eq!(first.list(), second.list(), "IDs must be stable across catalog order");
    assert_eq!(first.vault_id(), vault_id());
    // A local workspace path or identical filename cannot substitute for
    // the separately registered, typed identity of a different vault.
    let another_vault = VaultId::from_uuid(uuid::Uuid::from_u128(2));
    let different = ReadCatalog::open(
        &root, another_vault, &["alpha.md", "beta.md", "gamma.md"]
    ).unwrap();
    assert_ne!(id_of(&first, "alpha.md"), id_of(&different, "alpha.md"));
    assert_eq!(first.read(&id_of(&different, "alpha.md")).unwrap_err(),
        CatalogError::UnknownObject);
    assert_eq!(first.list().len(), 3);
    let unknown = "0000000000000000000000000000000000000000000000000000000000000000";
    assert_eq!(first.read(unknown).unwrap_err(), CatalogError::UnknownObject);
    assert!(!first.list().iter().any(|doc| doc.filename == "private.md"));

    for (name, bytes) in docs {
        let id = id_of(&first, name);
        assert_eq!(id.len(), 64, "never use unvalidated filenames as URL ids");
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let doc = first.read(&id).unwrap();
        assert_eq!(doc.id, id);
        assert_eq!(doc.content.as_bytes(), bytes, "do not normalize CRLF");
        assert_eq!(doc.raw_revision, blake3::hash(bytes).to_hex().to_string());
        assert_eq!(first.read_if_revision(&id, Some(&doc.raw_revision)).unwrap(), doc);
        assert_eq!(first.read_if_revision(&id, Some(unknown)).unwrap_err(),
            CatalogError::RevisionConflict);
    }
}

#[test]
fn invalid_names_duplicates_and_missing_entries_refuse_catalog_creation() {
    let (_scratch, root) = setup();
    fs::write(root.join("okay.md"), b"one").unwrap();
    invalid(&root, &[]);
    invalid(&root, &["okay.md", "okay.md"]);
    invalid(&root, &["../secret.md"]);
    invalid(&root, &["nested/okay.md"]);
    invalid(&root, &["folder\\secret.md"]);
    invalid(&root, &["./okay.md"]);
    invalid(&root, &["/etc/passwd.md"]);
    invalid(&root, &["okay.txt"]);
    invalid(&root, &["okay.md\n"]);
    invalid(&root, &["."]);
    invalid(&root, &["missing.md"]);
    invalid(&root, &["okay.md", "missing.md"]);
    invalid(&root, &["x".repeat(130).as_str()]);
    let names = vec!["okay.md"; MAX_DOCUMENTS + 1];
    invalid(&root, &names);

    fs::create_dir(root.join("folder.md")).unwrap();
    invalid(&root, &["folder.md"]);
    let outside = root.parent().unwrap().join("outside.md");
    fs::write(&outside, b"secret outside").unwrap();
    symlink(&outside, root.join("symlink.md")).unwrap();
    invalid(&root, &["symlink.md"]);

    let alias = root.parent().unwrap().join("root-alias");
    symlink(&root, &alias).unwrap();
    invalid(&alias, &["okay.md"]);
}

#[test]
fn original_root_fd_survives_path_replacement_but_file_identity_is_sticky() {
    let (scratch, root) = setup();
    fs::write(root.join("a.md"), b"original pinned root").unwrap();
    let catalog = ReadCatalog::open(&root, vault_id(), &["a.md"]).unwrap();
    let id = id_of(&catalog, "a.md");

    let old_root = scratch.path().join("old");
    fs::rename(&root, &old_root).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a.md"), b"attacker path replacement").unwrap();

    assert_eq!(catalog.read(&id).unwrap().content, "original pinned root");
    fs::rename(old_root.join("a.md"), old_root.join("retired.md")).unwrap();
    fs::write(old_root.join("a.md"), b"different inode in pinned root").unwrap();
    assert_eq!(catalog.read(&id).unwrap_err(), CatalogError::ReplacedObject);
}

#[test]
fn symlink_swap_and_new_file_revision_cannot_be_silently_accepted() {
    let (_scratch, root) = setup();
    let path = root.join("a.md");
    fs::write(&path, b"initial").unwrap();
    let catalog = ReadCatalog::open(&root, vault_id(), &["a.md"]).unwrap();
    let id = id_of(&catalog, "a.md");
    let previous_revision = catalog.read(&id).unwrap().raw_revision;

    fs::write(&path, b"modified in place").unwrap();
    assert_eq!(catalog.read_if_revision(&id, Some(&previous_revision)).unwrap_err(),
        CatalogError::RevisionConflict);
    assert_eq!(catalog.read(&id).unwrap().content, "modified in place");

    fs::remove_file(&path).unwrap();
    let outside = root.parent().unwrap().join("private.md");
    fs::write(&outside, b"SECRET").unwrap();
    symlink(&outside, &path).unwrap();
    assert_eq!(catalog.read(&id).unwrap_err(), CatalogError::ReplacedObject);
    assert_eq!(fs::read(&outside).unwrap(), b"SECRET");
}

#[test]
fn externally_pinned_fd_prevents_second_root_path_resolution() {
    use rustix::fs::{open, Mode, OFlags};
    use std::fs::File;

    let (scratch, root) = setup();
    fs::write(root.join("safe.md"), b"ORIGINAL trusted directory").unwrap();
    let pinned = File::from(
        open(&root, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
            | OFlags::CLOEXEC, Mode::empty()).unwrap()
    );
    // The original root path is replaced BEFORE catalog admission. A second
    // open(root_path) would bind a different directory, but the already
    // pinned File must bind the admitted catalog to the original inode.
    let retired = scratch.path().join("retired");
    fs::rename(&root, &retired).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("safe.md"), b"REPLACEMENT untrusted directory").unwrap();

    let catalog = ReadCatalog::from_pinned_root(
        pinned, vault_id(), &["safe.md"]
    ).unwrap();
    assert_eq!(catalog.read(&id_of(&catalog, "safe.md")).unwrap().content,
        "ORIGINAL trusted directory");
    assert_eq!(fs::read(root.join("safe.md")).unwrap(),
        b"REPLACEMENT untrusted directory");
}

#[test]
fn bounded_read_rejects_changed_sizes_and_invalid_utf8() {
    let (_scratch, root) = setup();
    let large = vec![b'X'; MAX_DOCUMENT_BYTES as usize];
    fs::write(root.join("limit.md"), &large).unwrap();
    fs::write(root.join("bad.md"), [0xff, 0xfe, 0xfd]).unwrap();
    let catalog = ReadCatalog::open(&root, vault_id(), &["limit.md", "bad.md"]).unwrap();

    let id = id_of(&catalog, "limit.md");
    assert_eq!(catalog.read(&id).unwrap().content.len() as u64, MAX_DOCUMENT_BYTES);
    fs::write(root.join("limit.md"), vec![b'Y'; MAX_DOCUMENT_BYTES as usize + 1]).unwrap();
    assert_eq!(catalog.read(&id).unwrap_err(), CatalogError::TooLarge);

    let bad_id = id_of(&catalog, "bad.md");
    assert_eq!(catalog.read(&bad_id).unwrap_err(), CatalogError::InvalidUtf8);

    let (_scratch2, another) = setup();
    fs::write(another.join("large.md"), vec![b'A'; MAX_DOCUMENT_BYTES as usize + 1]).unwrap();
    assert_eq!(ReadCatalog::open(&another, vault_id(), &["large.md"]).err(), Some(CatalogError::TooLarge));
}
