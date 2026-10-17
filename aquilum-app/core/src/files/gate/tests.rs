use super::moves::{notes_under, relocated};
use super::trash::removed;
use super::NoteMove;
use std::fs;

#[test]
fn a_note_moves_as_itself() {
    let directory = tempfile::tempdir().unwrap();
    let note = directory.path().join("Идея.md");
    fs::write(&note, "текст").unwrap();
    let target = directory.path().join("Проекты").join("Идея.md");

    let relocation = relocated(&notes_under(&note), &note, &target);

    assert_eq!(
        relocation.moves,
        vec![NoteMove {
            from: note.to_string_lossy().into_owned(),
            to: target.to_string_lossy().into_owned(),
        }]
    );
}

#[test]
fn a_text_file_moves_and_disappears_like_a_note() {
    let directory = tempfile::tempdir().unwrap();
    let text = directory.path().join("Черновик.txt");
    fs::write(&text, "текст").unwrap();
    let target = directory.path().join("Архив").join("Черновик.txt");

    let relocation = relocated(&notes_under(&text), &text, &target);
    assert_eq!(relocation.moves.len(), 1);
    assert_eq!(relocation.moves[0].to, target.to_string_lossy());
    assert_eq!(removed(notes_under(&text).iter()).removed, vec![text.to_string_lossy().into_owned()]);
}

#[test]
fn a_folder_moves_as_every_note_inside_it() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("Проекты");
    fs::create_dir_all(folder.join("Глубже")).unwrap();
    fs::write(folder.join("А.md"), "").unwrap();
    fs::write(folder.join("Глубже").join("Б.md"), "").unwrap();
    fs::write(folder.join("картинка.png"), "").unwrap();
    let target = directory.path().join("Архив");

    let mut relocation = relocated(&notes_under(&folder), &folder, &target);
    relocation.moves.sort_by(|left, right| left.from.cmp(&right.from));

    let targets = relocation
        .moves
        .iter()
        .map(|entry| entry.to.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![
            target.join("А.md").to_string_lossy().into_owned(),
            target.join("Глубже").join("Б.md").to_string_lossy().into_owned(),
        ],
        "только заметки, вложенность сохраняется"
    );
}

#[test]
fn a_deleted_folder_reports_the_notes_it_held() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("Старое");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("А.md"), "").unwrap();

    let relocation = removed(notes_under(&folder).iter());

    assert_eq!(relocation.removed, vec![folder.join("А.md").to_string_lossy().into_owned()]);
    assert!(relocation.moves.is_empty());
}


/// If another in-process File Gate operation holds the interlock, each real
/// operation must block until it is released. This does not protect against
/// writes by a separate process or authorize sync mutations.
#[test]
fn core_file_gate_serializes_create_write_copy_rename_trash_and_restore() {
    use crate::app_core::{Core, CoreEvent};
    use crate::files::error::FileCommandError;
    use crate::history::Source;
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;

    fn under_lock(
        core: &Arc<Core>,
        act: impl FnOnce(Arc<Core>) -> Result<(), FileCommandError> + Send + 'static,
    ) {
        let lock = core.file_mutations.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let core = Arc::clone(core);
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let result = act(core);
            done_tx.send(result).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "disk mutation completed while the Core File Gate lock was held");
        drop(lock);
        done_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        worker.join().unwrap();
    }

    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir(&vault).unwrap();
    let data = temp.path().join("appdata");
    fs::create_dir(&data).unwrap();
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
    let first: PathBuf = vault.join("first.md");
    let copy = vault.join("copy.md");
    let renamed = vault.join("renamed.md");

    let p = first.clone();
    under_lock(&core, move |core| {
        super::write::create(&core, &p, "original", Source::Me, None).map(|_| ())
    });
    assert_eq!(fs::read(&first).unwrap(), b"original");

    let p = first.clone();
    let old_hash = blake3::hash(b"original").to_hex().to_string();
    under_lock(&core, move |core| {
        super::write::write(&core, &p, "changed", Some(&old_hash), Source::Me, None).map(|_| ())
    });
    assert_eq!(fs::read(&first).unwrap(), b"changed");

    let from = first.clone();
    let to = copy.clone();
    under_lock(&core, move |core| super::write::copy(&core, &from, &to));
    assert_eq!(fs::read(&copy).unwrap(), b"changed");

    let from = first.clone();
    let to = renamed.clone();
    under_lock(&core, move |core| super::moves::rename(&core, &from, &to).map(|_| ()));
    assert!(!first.exists());
    assert_eq!(fs::read(&renamed).unwrap(), b"changed");

    // For trash and restore, use an explicit signal from the worker and
    // acquire the lock across the real move operation, not only an adapter.
    let root = vault.clone();
    let target = renamed.clone();
    let moved = Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let saved = Arc::clone(&moved);
    under_lock(&core, move |core| {
        let path = super::trash::trash(&core, &root, &target)?;
        *saved.lock().unwrap() = Some(path);
        Ok(())
    });
    let moved = moved.lock().unwrap().clone().unwrap();
    assert!(!renamed.exists());
    assert!(moved.exists());

    let root = vault.clone();
    under_lock(&core, move |core| {
        super::trash::restore(&core, &root, &moved).map(|_| ())
    });
    assert_eq!(fs::read(&renamed).unwrap(), b"changed");
}
