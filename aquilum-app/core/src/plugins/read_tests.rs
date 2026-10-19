use super::{
    Capability, GrantScope, HostContext, HostPluginVaultReader, NoteId,
    PermissionBroker, PluginManifest, PluginReadError, VaultId,
};
use crate::app_core::{Core, CoreEvent};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/plugins/fixtures/valid-v1.json"
));

struct Fixture {
    _temp: tempfile::TempDir,
    core: Arc<Core>,
    root: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let appdata = temp.path().join("appdata");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&appdata).unwrap();
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    Fixture { _temp: temp, core, root }
}

fn note(root: &Path, path: &str, bytes: &[u8]) {
    let file = root.join(path);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(file, bytes).unwrap();
}

fn plugin(
    broker: &PermissionBroker,
    vault: VaultId,
) -> super::PluginInstanceHandle {
    let manifest = PluginManifest::parse_json(MANIFEST).unwrap();
    broker.register_host_package(&manifest, [0x4a; 32], vault).unwrap()
}

#[test]
fn registered_workspace_note_is_readable_only_after_scoped_consent() {
    let f = fixture();
    note(&f.root, "Work/allowed.md", b"allowed contents");
    note(&f.root, "Work/other.md", b"hidden");
    note(&f.root, "WorkX/blocked.md", b"private");

    let broker = PermissionBroker::new();
    let reader = HostPluginVaultReader::open_registered(&f.core, &broker, &f.root).unwrap();
    let good = reader.bind_host_note("Work/allowed.md").unwrap();
    let other = reader.bind_host_note("Work/other.md").unwrap();
    let wrong_folder = reader.bind_host_note("WorkX/blocked.md").unwrap();
    let instance = plugin(&broker, reader.host_vault_id());
    let context = || HostContext::from_host_state(reader.host_vault_id(), Some(good.host_note_id()));

    assert_eq!(
        reader.read_note_bounded(instance, &good, 1024, context),
        Err(PluginReadError::Denied)
    );
    broker.grant_host_approved(
        instance, Capability::NotesRead, GrantScope::selected_notes([good.host_note_id()]).unwrap()
    ).unwrap();
    let result = reader.read_note_bounded(instance, &good, 1024, context).unwrap();
    assert_eq!(result.bytes, b"allowed contents");
    assert_eq!(result.content_hash, blake3::hash(b"allowed contents").to_hex().to_string());

    for blocked in [&other, &wrong_folder] {
        assert_eq!(
            reader.read_note_bounded(instance, blocked, 1024, context),
            Err(PluginReadError::Denied)
        );
    }
    broker.revoke_all_for_host(instance).unwrap();
    broker.grant_host_approved(
        instance, Capability::NotesRead, GrantScope::folder("Work").unwrap()
    ).unwrap();
    assert_eq!(
        reader.read_note_bounded(instance, &other, 1024, context).unwrap().bytes,
        b"hidden"
    );
    assert_eq!(
        reader.read_note_bounded(instance, &wrong_folder, 1024, context),
        Err(PluginReadError::Denied)
    );
}

#[test]
fn cannot_select_different_registered_vault_using_first_vault_consent() {
    let f = fixture();
    let other_root = f._temp.path().join("other");
    fs::create_dir(&other_root).unwrap();
    let other_ws = f.core.ui_state.resolve_workspace(other_root.to_str().unwrap(), 2).unwrap();
    note(&f.root, "Work/a.md", b"first vault");
    note(&other_root, "Work/a.md", b"second vault");

    let broker = PermissionBroker::new();
    let first = HostPluginVaultReader::open_registered(&f.core, &broker, &f.root).unwrap();
    let second = HostPluginVaultReader::open_registered(&f.core, &broker, &other_root).unwrap();
    assert_eq!(second.host_vault_id().as_uuid(), other_ws);
    assert_ne!(first.host_vault_id(), second.host_vault_id());

    let first_note = first.bind_host_note("Work/a.md").unwrap();
    let second_note = second.bind_host_note("Work/a.md").unwrap();
    let installed = plugin(&broker, first.host_vault_id());
    broker.grant_host_approved(
        installed, Capability::NotesRead, GrantScope::WholeVault
    ).unwrap();

    let first_ctx = || HostContext::from_host_state(first.host_vault_id(), Some(first_note.host_note_id()));
    let second_ctx = || HostContext::from_host_state(second.host_vault_id(), Some(second_note.host_note_id()));

    assert_eq!(first.read_note_bounded(installed, &first_note, 200, first_ctx).unwrap().bytes,b"first vault");
    assert_eq!(second.read_note_bounded(installed, &second_note, 200, second_ctx),Err(PluginReadError::Denied));
    assert_eq!(second.read_note_bounded(installed, &first_note, 200, second_ctx),Err(PluginReadError::Denied));
}

#[test]
fn missing_registration_and_revoked_workspace_id_fail_closed() {
    let f = fixture();
    note(&f.root, "n.md", b"safe");
    let broker = PermissionBroker::new();
    let unregistered = f._temp.path().join("unregistered");
    fs::create_dir(&unregistered).unwrap();
    assert!(matches!(
        HostPluginVaultReader::open_registered(&f.core, &broker, &unregistered),
        Err(PluginReadError::Unavailable)
    ));

    let reader = HostPluginVaultReader::open_registered(&f.core, &broker, &f.root).unwrap();
    let handle = reader.bind_host_note("n.md").unwrap();
    let installed = plugin(&broker, reader.host_vault_id());
    broker.grant_host_approved(installed,Capability::NotesRead,GrantScope::WholeVault).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),Some(handle.host_note_id()));
    let old_id = reader.host_vault_id().as_uuid();
    f.core.ui_state.forget_workspace(old_id).unwrap();
    assert_eq!(reader.read_note_bounded(installed,&handle,100,ctx),Err(PluginReadError::Unavailable));
    let new_id=f.core.ui_state.resolve_workspace(f.root.to_str().unwrap(),4).unwrap();
    assert_ne!(new_id,old_id);
    assert_eq!(reader.read_note_bounded(installed,&handle,100,ctx),Err(PluginReadError::Unavailable));
}

#[test]
fn stale_grant_or_selection_change_after_read_returns_no_bytes() {
    use std::cell::Cell;

    let f=fixture();
    note(&f.root,"Private/one.md",b"secret");
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    let target=reader.bind_host_note("Private/one.md").unwrap();
    let installed=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(installed,Capability::NotesRead,GrantScope::CurrentNote).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),Some(target.host_note_id()));
    assert_eq!(reader.read_with_hook(installed,&target,1024,ctx,||{
        broker.revoke_all_for_host(installed).unwrap();
    }),Err(PluginReadError::Denied));

    broker.grant_host_approved(installed,Capability::NotesRead,GrantScope::CurrentNote).unwrap();
    let active=Cell::new(Some(target.host_note_id()));
    let live=||HostContext::from_host_state(reader.host_vault_id(),active.get());
    assert_eq!(reader.read_with_hook(installed,&target,1024,live,||{
        active.set(Some(NoteId::new_for_host()));
    }),Err(PluginReadError::Denied));
}

#[test]
fn lexical_aliases_and_overlarge_reads_fail_before_exposing_content() {
    let f=fixture();
    note(&f.root,"Work/n.md",b"the secret is bounded");
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();

    for unsafe_path in [
        "../outside.md","/tmp/x.md","C:/Windows/win.ini","a\\b.md",
        "a/../b.md","a//b.md","a/./b.md","CON.md",
        "Work/name.","Work/aux.txt","Work/file:stream.md",
        "Work/cafe\u{301}.md","Work/x\nbad.md",
    ] {
        assert_eq!(reader.bind_host_note(unsafe_path).unwrap_err(),PluginReadError::InvalidHostPath,
            "{unsafe_path:?}");
    }

    let target=reader.bind_host_note("Work/n.md").unwrap();
    let inst=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(inst,Capability::NotesRead,GrantScope::WholeVault).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),None);
    assert_eq!(reader.read_note_bounded(inst,&target,2,ctx),Err(PluginReadError::Unavailable));
    assert_eq!(reader.read_note_bounded(inst,&target,0,ctx),Err(PluginReadError::Unavailable));
}

#[test]
fn a_hardlink_inside_a_granted_vault_never_exposes_the_outside_inode() {
    let f=fixture();
    let outside=f._temp.path().join("outside.md");
    fs::write(&outside,b"private file").unwrap();
    fs::hard_link(&outside,f.root.join("alias.md")).unwrap();
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    // The trusted binder never issues a handle for a file with >1 hard link.
    assert_eq!(reader.bind_host_note("alias.md").unwrap_err(),PluginReadError::Unavailable);
    assert_eq!(fs::read(outside).unwrap(),b"private file");
}

#[cfg(unix)]
#[test]
fn symlinked_parents_and_final_targets_are_rejected() {
    use std::os::unix::fs::symlink;
    let f=fixture();
    let outside=f._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private.md"),b"secret").unwrap();
    symlink(&outside,f.root.join("escape")).unwrap();
    symlink(outside.join("private.md"),f.root.join("alias.md")).unwrap();
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    for p in ["escape/private.md","alias.md"] {
        assert_eq!(reader.bind_host_note(p).unwrap_err(),PluginReadError::Unavailable);
    }
}

#[cfg(windows)]
#[test]
fn ntfs_junction_escape_is_rejected() {
    use std::process::Command;
    let f=fixture();
    let outside=f._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private.md"),b"secret").unwrap();
    let link=f.root.join("escape");
    let result=Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(&link).arg(&outside)
        .output().unwrap();
    assert!(result.status.success(),"cannot create fixture: {}",String::from_utf8_lossy(&result.stderr));
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    assert_eq!(reader.bind_host_note("escape/private.md").unwrap_err(),PluginReadError::Unavailable);
}

#[test]
fn workspace_root_replacement_does_not_switch_to_new_tree() {
    let f=fixture();
    note(&f.root,"n.md",b"old");
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    let handle=reader.bind_host_note("n.md").unwrap();
    let inst=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(inst,Capability::NotesRead,GrantScope::WholeVault).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),None);
    let moved=f._temp.path().join("original");
    fs::rename(&f.root,&moved).unwrap();
    fs::create_dir(&f.root).unwrap();
    note(&f.root,"n.md",b"substituted");
    assert_eq!(reader.read_note_bounded(inst,&handle,100,ctx),Err(PluginReadError::Unavailable));
    assert_eq!(fs::read(f.root.join("n.md")).unwrap(),b"substituted");
}

#[test]
fn open_editors_cannot_be_read_as_stale_disk_snapshots() {
    let f=fixture();
    note(&f.root,"n.md",b"disk before edit");
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    let handle=reader.bind_host_note("n.md").unwrap();
    let inst=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(inst,Capability::NotesRead,GrantScope::WholeVault).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),None);
    let disk=f.root.join("n.md");
    f.core.documents.open(&f.core,&disk,0).unwrap();
    assert_eq!(reader.read_note_bounded(inst,&handle,100,ctx),Err(PluginReadError::Unavailable));
    f.core.documents.release(&f.core,&disk);
    assert_eq!(reader.read_note_bounded(inst,&handle,100,ctx).unwrap().bytes,b"disk before edit");
}

#[test]
fn unbound_paths_are_not_exposed_by_a_missing_plugin_permission() {
    let f=fixture();
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    let inst=plugin(&broker,reader.host_vault_id());
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),None);
    assert_eq!(reader.bind_host_note("NotHere/unknown.md").unwrap_err(),PluginReadError::Unavailable);
    note(&f.root,"present.md",b"content is not returned without consent");
    let present=reader.bind_host_note("present.md").unwrap();
    assert_eq!(reader.read_note_bounded(inst,&present,2048,ctx),Err(PluginReadError::Denied));
}

#[test]
fn replacing_a_selected_file_with_identical_bytes_cannot_reuse_old_handle() {
    let f=fixture();
    note(&f.root,"Private/selected.md",b"same contents");
    let broker=PermissionBroker::new();
    let reader=HostPluginVaultReader::open_registered(&f.core,&broker,&f.root).unwrap();
    let selected=reader.bind_host_note("Private/selected.md").unwrap();
    let inst=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(inst,Capability::NotesRead,
        GrantScope::selected_notes([selected.host_note_id()]).unwrap()).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),Some(selected.host_note_id()));
    fs::rename(f.root.join("Private/selected.md"),f.root.join("Private/old.md")).unwrap();
    note(&f.root,"Private/selected.md",b"same contents");
    assert_eq!(reader.read_note_bounded(inst,&selected,200,ctx),Err(PluginReadError::Unavailable));
}

#[test]
fn pinned_note_binding_requires_read_grant_and_exact_sync_blob() {
    use crate::sync::{DocumentId, RevisionDag, RevisionId, RevisionOperation, RevisionRecord, SyncRelativePath};
    let f = fixture();
    note(&f.root, "notes/a.md", b"known content");
    let broker = PermissionBroker::new();
    let reader = HostPluginVaultReader::open_registered(&f.core, &broker, &f.root).unwrap();
    let handle = reader.bind_host_note("notes/a.md").unwrap();
    let installed = plugin(&broker, reader.host_vault_id());
    let live = || HostContext::from_host_state(reader.host_vault_id(), Some(handle.host_note_id()));
    let document = DocumentId::new();
    let mut dag = RevisionDag::new();
    let current = RevisionId::new();
    dag.ingest(RevisionRecord {
        id: current, document_id: document, parents: vec![],
        operation: RevisionOperation::Content {
            path: SyncRelativePath::parse("notes/a.md").unwrap(),
            blob_hash: blake3::hash(b"known content").to_hex().to_string(),
        },
    }).unwrap();
    assert!(reader.bind_current_sync_note(installed, &handle, &dag, 2048, live).is_err());
    broker.grant_host_approved(installed, Capability::NotesRead, GrantScope::WholeVault).unwrap();
    assert!(reader.bind_current_sync_note(installed, &handle, &dag, 2048, live).is_ok());
    note(&f.root, "notes/a.md", b"changed content");
    assert!(reader.bind_current_sync_note(installed, &handle, &dag, 2048, live).is_err());
}

#[test]
fn pinned_note_binding_rejects_other_dag_path_and_stale_instance() {
    use crate::sync::{DocumentId, RevisionDag, RevisionId, RevisionOperation, RevisionRecord, SyncRelativePath};
    let f = fixture();
    note(&f.root, "notes/a.md", b"known content");
    let broker = PermissionBroker::new();
    let reader = HostPluginVaultReader::open_registered(&f.core, &broker, &f.root).unwrap();
    let handle = reader.bind_host_note("notes/a.md").unwrap();
    let installed = plugin(&broker, reader.host_vault_id());
    broker.grant_host_approved(installed, Capability::NotesRead, GrantScope::WholeVault).unwrap();
    let live = || HostContext::from_host_state(reader.host_vault_id(), Some(handle.host_note_id()));
    let mut dag = RevisionDag::new();
    dag.ingest(RevisionRecord {
        id: RevisionId::new(), document_id: DocumentId::new(), parents: vec![],
        operation: RevisionOperation::Content {
            path: SyncRelativePath::parse("notes/other.md").unwrap(),
            blob_hash: blake3::hash(b"known content").to_hex().to_string(),
        },
    }).unwrap();
    assert!(reader.bind_current_sync_note(installed, &handle, &dag, 2048, live).is_err());
    broker.revoke_all_for_host(installed).unwrap();
    assert!(reader.bind_current_sync_note(installed, &handle, &dag, 2048, live).is_err());
}

#[test]
fn pinned_note_to_sync_to_vector_search_respects_separate_grants() {
    use crate::search::semantic_probe::{
        self, Chunk, ProbeError,
    };
    use crate::sync::{DocumentId, RevisionDag, RevisionId, RevisionOperation, RevisionRecord, SyncRelativePath};
    let f = fixture();
    let bytes = b"verified document bytes";
    note(&f.root, "notes/e2e.md", bytes);
    let broker = PermissionBroker::new();
    let reader = HostPluginVaultReader::open_registered(&f.core, &broker, &f.root).unwrap();
    let handle = reader.bind_host_note("notes/e2e.md").unwrap();

    // Fixture declares only notes.read. Search authority is deliberately
    // separate and cannot be inferred from a successful protected file read.
    let installed = plugin(&broker, reader.host_vault_id());
    broker.grant_host_approved(installed, Capability::NotesRead, GrantScope::WholeVault).unwrap();
    let live = || HostContext::from_host_state(reader.host_vault_id(), Some(handle.host_note_id()));

    let doc = DocumentId::new();
    let rev = RevisionId::new();
    let mut dag = RevisionDag::new();
    dag.ingest(RevisionRecord {
        id: rev, document_id: doc, parents: vec![],
        operation: RevisionOperation::Content {
            path: SyncRelativePath::parse("notes/e2e.md").unwrap(),
            blob_hash: blake3::hash(bytes).to_hex().to_string(),
        },
    }).unwrap();
    let selection = reader.bind_current_sync_note(installed, &handle, &dag, 4096, live).unwrap();
    assert_eq!(selection.sync_document_id().unwrap(), doc);

    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    semantic_probe::prepare(&db).unwrap();
    let scope = reader.host_vault_id().as_uuid().to_string();
    semantic_probe::register_source(&db, &scope, &doc.to_string()).unwrap();
    let source_hash = blake3::hash(bytes).to_hex().to_string();
    let text_hash = blake3::hash(b"fragment").to_hex().to_string();
    semantic_probe::replace_revision_chunks(
        &mut db, &scope, &doc.to_string(), 1, &rev.to_string(), &source_hash,
        "e2e-fixture@1", 2,
        &[Chunk {locator: "p:1", text_hash: &text_hash, embedding: &[1.0, 0.0]}],
    ).unwrap();

    // The verified selection is enough for an internal owner query, but NOT
    // for any plugin disclosure without an independent NotesSearch lease.
    let local = semantic_probe::search_sync_verified(
        &db, &scope, 1, &selection, "e2e-fixture@1", &[1.0, 0.0], 5,
    ).unwrap();
    assert_eq!(local.hits.len(), 1);
    assert_eq!(local.hits[0].revision_id, rev.to_string());
    assert_eq!(local.hits[0].source_hash, source_hash);

    // Altering the file invalidates a new protected binding, even when the
    // previously generated vector cache still contains old results.
    note(&f.root, "notes/e2e.md", b"modified after indexing");
    assert!(reader.bind_current_sync_note(installed, &handle, &dag, 4096, live).is_err());
    assert!(matches!(
        semantic_probe::search_sync_verified(
            &db, &scope, 2, &selection, "e2e-fixture@1", &[1.0, 0.0], 5,
        ),
        Err(ProbeError::StaleSource)
    ));
}
