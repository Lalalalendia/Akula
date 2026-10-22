//! PLUGIN-05D: real WASM Component guest + owned WIT note resource,
//! backed by Akula's existing registered vault reader and PermissionBroker.
//! The guest never receives the Core identity, path, authority, or file bytes.
use super::*;
use aquilum_core::{
    app_core::{Core, CoreEvent},
    plugins::{
        Capability, GrantScope, HostContext, HostPluginVaultReader, NoteId, NoteResource,
        PermissionBroker, PluginInstanceHandle, PluginManifest, Resource as PermissionResource,
        ScopedNoteHandle, VaultId,
    },
};
use std::{fs, path::PathBuf, sync::Arc};

const MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../docs/plugins/fixtures/valid-v1.json"
));

struct RealResourceHost {
    table: ResourceTable,
    core: Arc<Core>,
    broker: Arc<PermissionBroker>,
    root: PathBuf,
    instance: PluginInstanceHandle,
    vault: VaultId,
    selected: Option<NoteId>,
    selected_handle: ScopedNoteHandle,
    instance_generation: u64,
    vault_generation: u64,
    handle_epoch: u64,
    drops: usize,
}

impl RealResourceHost {
    fn reader(&self) -> Result<HostPluginVaultReader<'_>, ()> {
        HostPluginVaultReader::open_registered(
            self.core.as_ref(),
            self.broker.as_ref(),
            &self.root,
        ).map_err(|_| ())
    }

    fn context(&self) -> HostContext {
        // The host's UI selection, not a message supplied by the guest.
        HostContext::from_host_state(self.vault, self.selected)
    }

    fn bounded_read(&self, instance: PluginInstanceHandle, note: &ScopedNoteHandle) -> i32 {
        let Ok(reader) = self.reader() else {
            return -1;
        };
        let context = || self.context();
        match reader.read_note_bounded(instance, note, 1024, context) {
            Ok(snapshot) => snapshot.bytes.first().map_or(0, |byte| i32::from(*byte)),
            Err(_) => -1,
        }
    }
}

impl Host for RealResourceHost {
    fn open_active(&mut self) -> wasmtime::Result<Option<Resource<BoundNote>>> {
        // A missing grant must not mint even an opaque owned Resource.
        if self.bounded_read(self.instance, &self.selected_handle) < 0 {
            return Ok(None);
        }
        // The host-selected indexed path is NOT supplied by the guest.
        // Capture an immutable revision-bound lease in the owned WIT handle.
        let Ok(target) = NoteResource::from_host_index(
            self.vault,
            self.selected_handle.host_note_id(),
            "active.md",
        ) else {
            return Ok(None);
        };
        let Ok(lease) = self.broker.authorize(
            self.instance,
            self.context(),
            Capability::NotesRead,
            &PermissionResource::Note(target),
        ) else {
            return Ok(None);
        };
        let entry = BoundNote {
            instance: self.instance_generation,
            vault: self.vault_generation,
            epoch: self.handle_epoch,
            actual: Some(CoreBoundNote {
                instance: self.instance,
                vault: self.vault,
                note: self.selected_handle.clone(),
                lease,
            }),
        };
        Ok(Some(self.table.push(entry)?))
    }
}

impl HostNote for RealResourceHost {
    fn read_first(&mut self, note: Resource<BoundNote>) -> wasmtime::Result<i32> {
        // WIT ResourceTable owns the entry: guest-controlled integers/paths
        // never enter either the table lookup or the permission decision.
        let entry = self.table.get(&note)?;
        if entry.instance != self.instance_generation
            || entry.vault != self.vault_generation
            || entry.epoch != self.handle_epoch
        {
            return Ok(-1);
        }
        let Some(binding) = entry.actual.as_ref() else {
            return Ok(-1);
        };
        if binding.instance != self.instance || binding.vault != self.vault {
            return Ok(-1);
        }
        // This is the ORIGINAL authorization revision, not a fresh grant.
        // Revocation invalidates old handles even after a re-grant, without
        // any synthetic generation update in the host.
        if self.broker.revalidate_lease(&binding.lease, self.context()).is_err() {
            return Ok(-1);
        }
        let value = self.bounded_read(binding.instance, &binding.note);
        // Recheck after the pinned read, before releasing even the first byte.
        if self.broker.revalidate_lease(&binding.lease, self.context()).is_err() {
            return Ok(-1);
        }
        Ok(value)
    }

    fn drop(&mut self, note: Resource<BoundNote>) -> wasmtime::Result<()> {
        self.table.delete(note)?;
        self.drops += 1;
        Ok(())
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    core: Arc<Core>,
    broker: Arc<PermissionBroker>,
    instance: PluginInstanceHandle,
    note_path: PathBuf,
    vault: VaultId,
    selected: NoteId,
    root: PathBuf,
    note: ScopedNoteHandle,
}

fn fixture() -> LabResult<Fixture> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("vault");
    let appdata = temporary.path().join("appdata");
    fs::create_dir(&root)?;
    fs::create_dir(&appdata)?;
    let note_path = root.join("active.md");
    fs::write(&note_path, b"A real note")?;
    fs::write(root.join("other.md"), b"B private")?;
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state
        .resolve_workspace(root.to_str().ok_or("invalid utf8 root")?, 1)
        .expect("registered test vault required");
    let broker = Arc::new(PermissionBroker::new());
    let reader = HostPluginVaultReader::open_registered(&core, &broker, &root)
        .expect("test vault admission required");
    let note = reader.bind_host_note("active.md")
        .expect("trusted host must pin active.md");
    let vault = reader.host_vault_id();
    let selected = note.host_note_id();
    let manifest = PluginManifest::parse_json(MANIFEST)?;
    let instance = broker.register_host_package(&manifest, [0x4a; 32], vault)?;
    Ok(Fixture {
        _temporary: temporary, core, broker, instance, note_path, vault, selected, root, note,
    })
}

fn harness_with_core(f: &Fixture) -> LabResult<(Store<RealResourceHost>, ResourceLab)> {
    let engine = engine()?;
    let component = component(&engine)?;
    let mut store = Store::new(&engine, RealResourceHost {
        table: ResourceTable::new(),
        core: Arc::clone(&f.core),
        broker: Arc::clone(&f.broker),
        root: f.root.clone(),
        instance: f.instance,
        vault: f.vault,
        selected: Some(f.selected),
        selected_handle: f.note.clone(),
        instance_generation: 1,
        vault_generation: 1,
        handle_epoch: 1,
        drops: 0,
    });
    store.set_fuel(10_000_000)?;
    let mut linker: Linker<RealResourceHost> = Linker::new(&engine);
    ResourceLab::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)?;
    let guest = ResourceLab::instantiate(&mut store, &component, &linker)?;
    Ok((store, guest))
}

#[test]
fn real_broker_controls_owned_wit_resource_and_revocation() -> LabResult<()> {
    let f = fixture()?;
    let (mut store, guest) = harness_with_core(&f)?;
    assert!(!guest.call_acquire(&mut store)?, "no permission must mint no handle");
    assert_eq!(guest.call_read_held(&mut store)?, -1);
    f.broker.grant_host_approved(f.instance, Capability::NotesRead, GrantScope::CurrentNote)?;
    assert!(guest.call_acquire(&mut store)?);
    assert_eq!(guest.call_read_held(&mut store)?, 65);
    let before = fs::read(&f.note_path)?;

    // The existing CurrentNote broker policy is evaluated on each read.
    store.data_mut().selected = Some(NoteId::new_for_host());
    assert_eq!(guest.call_read_held(&mut store)?, -1);
    store.data_mut().selected = Some(f.selected);
    assert_eq!(guest.call_read_held(&mut store)?, 65);

    f.broker.revoke_all_for_host(f.instance)?;
    assert_eq!(guest.call_read_held(&mut store)?, -1);

    // No synthetic epoch increment: the real broker's stored lease revision
    // must make the OLD guest resource permanently stale after re-grant.
    f.broker.grant_host_approved(f.instance, Capability::NotesRead, GrantScope::CurrentNote)?;
    assert_eq!(
        guest.call_read_held(&mut store)?,
        -1,
        "re-grant must never resurrect the prior broker lease",
    );
    guest.call_release(&mut store)?;
    assert_eq!(store.data().drops, 1);
    assert!(guest.call_acquire(&mut store)?);
    assert_eq!(guest.call_read_held(&mut store)?, 65);
    guest.call_release(&mut store)?;
    assert_eq!(store.data().drops, 2);

    assert_eq!(fs::read(&f.note_path)?, before, "guest cannot modify the note");
    f.core.shutdown();
    Ok(())
}

#[test]
fn changed_instance_vault_and_replaced_file_cannot_reuse_resource() -> LabResult<()> {
    let f = fixture()?;
    f.broker.grant_host_approved(f.instance, Capability::NotesRead, GrantScope::CurrentNote)?;
    let (mut store, guest) = harness_with_core(&f)?;
    assert!(guest.call_acquire(&mut store)?);
    assert_eq!(guest.call_read_held(&mut store)?, 65);

    store.data_mut().instance_generation += 1;
    assert_eq!(guest.call_read_held(&mut store)?, -1);
    store.data_mut().instance_generation -= 1;
    store.data_mut().vault_generation += 1;
    assert_eq!(guest.call_read_held(&mut store)?, -1);
    store.data_mut().vault_generation -= 1;

    // The existing pinned File Identity guarantees that path replacement
    // never yields data from a different inode, even if bytes match.
    let backup = f.note_path.with_extension("backup");
    fs::rename(&f.note_path, &backup)?;
    fs::write(&f.note_path, b"X substituted")?;
    assert_eq!(guest.call_read_held(&mut store)?, -1);
    guest.call_release(&mut store)?;
    assert_eq!(store.data().drops, 1);
    assert_eq!(fs::read(&f.note_path)?, b"X substituted");
    f.core.shutdown();
    Ok(())
}

#[test]
fn one_hundred_guest_owned_resources_are_reclaimed() -> LabResult<()> {
    let f = fixture()?;
    f.broker.grant_host_approved(f.instance, Capability::NotesRead, GrantScope::CurrentNote)?;
    let (mut store, guest) = harness_with_core(&f)?;
    for i in 1..=100 {
        assert!(guest.call_acquire(&mut store)?);
        assert_eq!(guest.call_read_held(&mut store)?, 65);
        guest.call_release(&mut store)?;
        assert_eq!(store.data().drops, i);
    }
    f.core.shutdown();
    Ok(())
}
