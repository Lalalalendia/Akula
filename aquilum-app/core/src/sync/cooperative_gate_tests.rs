//! Two real processes exercise the REAL File Gate write + HistoryService
//! while one existing external OS lock stays held. Fixture-only cooperative
//! experiment: it does NOT make normal Akula writers interprocess-safe.

use super::filegate_custody_identity::SqliteCustodyIdentityStore;
use super::filegate_lock_custody::scoped_leaf;
use super::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
use super::{RegisteredReadAdmission, VaultId};
use crate::app_core::{Core, CoreEvent};
use crate::files::gate;
use crate::history::Source;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const CHILD_ENTRYPOINT: &str = "sync::cooperative_gate_tests::oslock_cooperative_child_entrypoint";
const CHILD_MARKER: &str = "AKULA_COOPERATIVE_FILEGATE_TEST_CHILD";

fn core_at(data: &Path, root: &Path) -> Arc<Core> {
    fs::create_dir_all(data).unwrap();
    let core = Core::open(data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    core
}

fn anchor_for(temp: &tempfile::TempDir, root: &Path) -> PathBuf {
    let root_id = PinnedProbeRoot::open(root).unwrap().identity();
    let scope = temp.path().join("external-existing-custody").join(scoped_leaf(root_id));
    fs::create_dir_all(&scope).unwrap();
    let leaf = scope.join(LOCK_NAME);
    fs::write(&leaf, b"cooperative-test-existing-lock").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&leaf, fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(unix)]
    { fs::canonicalize(scope).unwrap() }
    #[cfg(windows)]
    { scope }
}

fn until_child_reports(child: &mut Child, path: &Path, expected: &[u8]) {
    let limit = Instant::now() + Duration::from_secs(20);
    loop {
        if path.exists() {
            assert_eq!(fs::read(path).unwrap(), expected);
            return;
        }
        assert!(child.try_wait().unwrap().is_none(), "child exited before reporting");
        assert!(Instant::now() < limit, "child OS-lock wait timed out");
        thread::sleep(Duration::from_millis(15));
    }
}

struct KillOnDrop(Child);
impl KillOnDrop {
    fn terminate(&mut self) {
        let _ = self.0.kill();
        self.0.wait().unwrap();
    }
}
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn oslock_cooperative_child_entrypoint() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("AKULA_COOP_ROOT").unwrap());
    let data = PathBuf::from(std::env::var_os("AKULA_COOP_DATA").unwrap());
    let scope = PathBuf::from(std::env::var_os("AKULA_COOP_SCOPE").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_COOP_REPORT").unwrap());
    let vault = std::env::var("AKULA_COOP_VAULT").unwrap().parse::<VaultId>().unwrap();
    let core = core_at(&data, &root);
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let store = SqliteCustodyIdentityStore::open(&admission, &data.join("receipt.sqlite")).unwrap();

    store.with_cooperative_writer_lifetime(&admission, vault, &scope, || {
        fs::write(report.with_extension("held"), b"held").unwrap();
        let go = report.with_extension("go");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !go.exists() {
            assert!(Instant::now() < deadline, "parent did not release write barrier");
            thread::sleep(Duration::from_millis(10));
        }

        // THIS IS THE REAL ordinary File Gate path, which itself takes and
        // releases Core's mutex before writing durable HistoryService bytes.
        let note = root.join("Shared.md");
        gate::write(
            &core, &note, &"child edit ".repeat(90), None,
            Source::Me, Some("cooperative-child"),
        ).unwrap();
        let history = core.history.page(&|| vec![root.clone()], &note, 0, 8);
        assert!(history.total > 0, "File Gate returned before durable history record");
        fs::write(&report, b"saved-with-history").unwrap();

        // Remain alive with OS lock held AFTER both writes. The second Core
        // must be unable to begin another write until we are killed.
        loop { thread::park_timeout(Duration::from_secs(60)); }
    }).unwrap();
}

#[test]
fn oslock_cooperative_two_process_filegate_and_history_stay_serialized_through_kill() {
    let temp = tempfile::tempdir().unwrap();
    let selected_root = temp.path().join("shared-vault");
    fs::create_dir(&selected_root).unwrap();
    // UI-state registers a canonical physical root. HistoryService uses
    // lexical strip_root, so /var and /private/var must not be mixed on
    // macOS (Windows also normalizes extended-length path prefixes).
    let root = crate::search::paths::canonical_path(&selected_root);
    let note = root.join("Shared.md");
    fs::write(&note, b"original-untouched").unwrap();
    let data_a = temp.path().join("core-a");
    let data_b = temp.path().join("core-b");
    let scope = anchor_for(&temp, &root);
    let vault_id = VaultId::new();

    let parent_core = core_at(&data_a, &root);
    let child_setup = core_at(&data_b, &root);
    {
        let admitted = RegisteredReadAdmission::open(&child_setup, &root).unwrap();
        let mut receipt = SqliteCustodyIdentityStore::open(
            &admitted, &data_b.join("receipt.sqlite"),
        ).unwrap();
        receipt.bind_observed(&admitted, vault_id, &scope).unwrap();
    }
    drop(child_setup);
    let admitted = RegisteredReadAdmission::open(&parent_core, &root).unwrap();
    let mut receipt = SqliteCustodyIdentityStore::open(
        &admitted, &data_a.join("receipt.sqlite"),
    ).unwrap();
    receipt.bind_observed(&admitted, vault_id, &scope).unwrap();

    // A custody receipt must never make writes outside the registered root,
    // to a mismatched VaultId, or via another Core's registration possible.
    let outside = temp.path().join("outside.md");
    fs::write(&outside, b"leave outside untouched").unwrap();
    assert!(gate::write_with_registered_custody(
        &parent_core, &admitted, &receipt, vault_id, &scope, &outside,
        "forbidden", None, Source::Me, None,
    ).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"leave outside untouched");
    assert!(gate::write_with_registered_custody(
        &parent_core, &admitted, &receipt, VaultId::new(), &scope, &note,
        "forbidden", None, Source::Me, None,
    ).is_err());
    let impostor = core_at(&temp.path().join("impostor-appdata"), &root);
    assert!(gate::write_with_registered_custody(
        &impostor, &admitted, &receipt, vault_id, &scope, &note,
        "wrong Core", None, Source::Me, None,
    ).is_err());
    assert_eq!(fs::read(&note).unwrap(), b"original-untouched");

    let report = temp.path().join("cooperative-report.txt");
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD_ENTRYPOINT).arg("--nocapture")
        .env(CHILD_MARKER, "yes")
        .env("AKULA_COOP_ROOT", &root)
        .env("AKULA_COOP_DATA", &data_b)
        .env("AKULA_COOP_SCOPE", &scope)
        .env("AKULA_COOP_VAULT", vault_id.to_string())
        .env("AKULA_COOP_REPORT", &report)
        .spawn().unwrap();
    let mut child = KillOnDrop(child);
    until_child_reports(&mut child.0, &report.with_extension("held"), b"held");

    // Exercise the NEW opt-in production writer, not just a test wrapper.
    // A real second process already owns the same physical OS lock.
    let contested = gate::write_with_registered_custody(
        &parent_core, &admitted, &receipt, vault_id, &scope, &note,
        "must-never-be-written", None, Source::Me, None,
    );
    assert!(matches!(contested, Err(crate::files::error::FileCommandError::Task { .. })));
    assert_eq!(fs::read(&note).unwrap(), b"original-untouched");

    fs::write(report.with_extension("go"), b"go").unwrap();
    until_child_reports(&mut child.0, &report, b"saved-with-history");
    assert_eq!(fs::read_to_string(&note).unwrap(), "child edit ".repeat(90));
    let history = parent_core.history.page(&|| vec![root.clone()], &note, 0, 8);
    assert!(history.total > 0, "other Core cannot see child HistoryService writes");
    assert!(child.0.try_wait().unwrap().is_none(), "child must still hold OS lock");

    // Even AFTER durable history and primary bytes, the child OS lock stays
    // held; a second writer must still refuse without modifying either.
    let again = gate::write_with_registered_custody(
        &parent_core, &admitted, &receipt, vault_id, &scope, &note,
        "still-denied", None, Source::Me, None,
    );
    assert!(again.is_err());
    assert_eq!(fs::read_to_string(&note).unwrap(), "child edit ".repeat(90));
    child.terminate();

    // A hard kill releases the OS lock. A second independently registered
    // Core can now perform BOTH primary save and history work under the
    // SAME existing physical lock object.
    let result = gate::write_with_registered_custody(
        &parent_core, &admitted, &receipt, vault_id, &scope, &note,
        &"parent edit ".repeat(90), None, Source::Me, Some("cooperative-parent"),
    ).unwrap();
    assert_eq!(result.hash, blake3::hash("parent edit ".repeat(90).as_bytes()).to_hex().to_string());
    assert!(parent_core.history.page(&|| vec![root.clone()], &note, 0, 8).total > 0);
    assert_eq!(fs::read_to_string(&note).unwrap(), "parent edit ".repeat(90));
    assert_eq!(
        fs::read(scope.join(LOCK_NAME)).unwrap(),
        b"cooperative-test-existing-lock",
    );
}

#[test]
fn guarded_note_write_notifies_host_after_releasing_external_os_lock() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let selected_root = temp.path().join("registered-vault");
    fs::create_dir(&selected_root).unwrap();
    let root = crate::search::paths::canonical_path(&selected_root);
    let note = root.join("Guarded.md");
    fs::write(&note, b"existing-not-overwritten-until-admitted").unwrap();
    let scope = anchor_for(&temp, &root);
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(&appdata).unwrap();

    let event_seen = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&event_seen);
    let for_callback = scope.clone();
    let core = Core::open(&appdata, Arc::new(move |event: CoreEvent| {
        if matches!(event, CoreEvent::NoteHistoryChanged(_)) {
            // A real synchronous UI callback may launch another operation.
            // Holding the non-recursive external OS lock here would deadlock
            // or cause a false "busy" refusal.
            let pin = PinnedProbeRoot::open(&for_callback).unwrap();
            let guard = pin.try_lock().unwrap();
            assert!(guard.is_some(), "host callback executed under held OS lock");
            seen.store(true, Ordering::SeqCst);
        }
    }));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let admitted = RegisteredReadAdmission::open(&core, &root).unwrap();
    let vault = VaultId::new();
    let mut receipt = SqliteCustodyIdentityStore::open(
        &admitted, &appdata.join("custody.sqlite"),
    ).unwrap();
    receipt.bind_observed(&admitted, vault, &scope).unwrap();
    let content = "new content with substantial history ".repeat(100);
    gate::write_with_registered_custody(
        &core, &admitted, &receipt, vault, &scope,
        &note, &content, None, Source::Me, Some("guarded"),
    ).unwrap();
    assert_eq!(fs::read_to_string(&note).unwrap(), content);
    assert!(core.history.page(&|| vec![root.clone()], &note, 0, 8).total > 0);
    assert!(event_seen.load(Ordering::SeqCst),
        "successful history persistence must announce after OS release");

    // Reject an aliased parent with the same reachable bytes; the external
    // custody identity does not authorize following note-folder symlinks.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::create_dir(root.join("genuine")).unwrap();
        fs::write(root.join("genuine").join("Other.md"), b"still safe").unwrap();
        symlink(root.join("genuine"), root.join("alias")).unwrap();
        assert!(gate::write_with_registered_custody(
            &core, &admitted, &receipt, vault, &scope,
            &root.join("alias").join("Other.md"), "forbidden",
            None, Source::Me, None,
        ).is_err());
        assert_eq!(
            fs::read(root.join("genuine").join("Other.md")).unwrap(),
            b"still safe"
        );
    }
}

#[test]
fn oslock_cooperative_held_receipt_rejects_a_replaced_lock_before_any_closure() {
    let temp = tempfile::tempdir().unwrap();
    let selected_root = temp.path().join("vault");
    fs::create_dir(&selected_root).unwrap();
    // UI-state registers a canonical physical root. HistoryService uses
    // lexical strip_root, so /var and /private/var must not be mixed on
    // macOS (Windows also normalizes extended-length path prefixes).
    let root = crate::search::paths::canonical_path(&selected_root);
    let core = core_at(&temp.path().join("appdata"), &root);
    let scope = anchor_for(&temp, &root);
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let mut store = SqliteCustodyIdentityStore::open(
        &admission, &temp.path().join("appdata").join("receipt.sqlite"),
    ).unwrap();
    let vault = VaultId::new();
    store.bind_observed(&admission, vault, &scope).unwrap();
    let old = scope.join("old-lock-retained");
    let lock = scope.join(LOCK_NAME);
    fs::rename(&lock, &old).unwrap();
    fs::write(&lock, b"forged-new-lock").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let invoked = std::cell::Cell::new(false);
    assert!(store.with_cooperative_writer_lifetime(&admission, vault, &scope, || {
        invoked.set(true);
    }).is_err());
    assert!(!invoked.get());
    assert_eq!(fs::read(&old).unwrap(), b"cooperative-test-existing-lock");
    assert_eq!(fs::read(&lock).unwrap(), b"forged-new-lock");
}
