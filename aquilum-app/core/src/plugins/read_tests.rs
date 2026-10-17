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
    let handle=reader.bind_host_note("alias.md").unwrap();
    let inst=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(inst,Capability::NotesRead,GrantScope::WholeVault).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),None);
    assert_eq!(reader.read_note_bounded(inst,&handle,100,ctx),Err(PluginReadError::Unavailable));
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
    let inst=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(inst,Capability::NotesRead,GrantScope::WholeVault).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),None);
    for p in ["escape/private.md","alias.md"] {
        let handle=reader.bind_host_note(p).unwrap();
        assert_eq!(reader.read_note_bounded(inst,&handle,100,ctx),Err(PluginReadError::Unavailable));
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
    let target=reader.bind_host_note("escape/private.md").unwrap();
    let inst=plugin(&broker,reader.host_vault_id());
    broker.grant_host_approved(inst,Capability::NotesRead,GrantScope::WholeVault).unwrap();
    let ctx=||HostContext::from_host_state(reader.host_vault_id(),None);
    assert_eq!(reader.read_note_bounded(inst,&target,4096,ctx),Err(PluginReadError::Unavailable));
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
