//! PLUGIN-05C falsifier: a real WIT guest calls the already-authoritative
//! PLUGIN-02/04 PermissionBroker and registered pinned vault reader.
//! TEST-ONLY path dependency; no runtime is linked into the installed Akula.
use super::*;
use aquilum_core::{
    app_core::{Core, CoreEvent},
    plugins::{
        Capability, GrantScope, HostContext, HostPluginVaultReader, NoteId,
        PermissionBroker, PluginInstanceHandle, PluginManifest, ScopedNoteHandle, VaultId,
    },
};
use std::{fs, path::PathBuf, sync::Arc};

const MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../docs/plugins/fixtures/valid-v1.json"
));

struct BrokerHost {
    core: Arc<Core>,
    broker: Arc<PermissionBroker>,
    vault_root: PathBuf,
    instance: PluginInstanceHandle,
    note: ScopedNoteHandle,
    vault: VaultId,
    selected_note: Option<NoteId>,
}

impl LabImports for BrokerHost {
    fn read_scoped(&mut self, token: u32) -> i32 {
        // The guest controls only a lookup token, not a host path, vault ID,
        // plugin identity, grant epoch or a direct operating-system handle.
        if token != 7 {
            return -1;
        }
        let Ok(reader) = HostPluginVaultReader::open_registered(
            self.core.as_ref(),
            self.broker.as_ref(),
            &self.vault_root,
        ) else {
            return -1;
        };
        let context = || HostContext::from_host_state(self.vault, self.selected_note);
        match reader.read_note_bounded(self.instance, &self.note, 1024, context) {
            Ok(snapshot) => snapshot.bytes.first().map_or(0, |byte| i32::from(*byte)),
            Err(_) => -1,
        }
    }
}

#[test]
fn real_plugin_broker_grants_one_note_then_denies_revoked_guest() -> LabResult<()> {
    let temporary = tempfile::tempdir()?;
    let vault_root = temporary.path().join("vault");
    let appdata = temporary.path().join("appdata");
    fs::create_dir(&vault_root)?;
    fs::create_dir(&appdata)?;
    let note_path = vault_root.join("allowed.md");
    fs::write(&note_path, b"A confidential note")?;
    fs::write(vault_root.join("other.md"), b"B private")?;

    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(
        vault_root.to_str().ok_or("vault path not UTF-8")?,
        1,
    ).expect("test vault must register");
    let broker = Arc::new(PermissionBroker::new());
    let reader = HostPluginVaultReader::open_registered(&core, &broker, &vault_root)
        .expect("test vault admission must succeed");
    let note = reader.bind_host_note("allowed.md")
        .expect("host-selected note must pin");
    let vault = reader.host_vault_id();
    let id = note.host_note_id();
    let manifest = PluginManifest::parse_json(MANIFEST)?;
    let instance = broker.register_host_package(&manifest, [0x4a; 32], vault)?;

    let engine = engine()?;
    let (component, _) = compiled_component(&engine)?;
    let mut linker: Linker<BrokerHost> = Linker::new(&engine);
    Lab::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)?;
    let mut store = Store::new(
        &engine,
        BrokerHost {
            core: Arc::clone(&core),
            broker: Arc::clone(&broker),
            vault_root,
            instance,
            note,
            vault,
            selected_note: Some(id),
        },
    );
    store.set_fuel(2_000_000)?;
    let guest = Lab::instantiate(&mut store, &component, &linker)?;

    // Deny by default; arbitrary/foreign guest token never names a real path.
    assert_eq!(guest.call_hello(&mut store)?, 42);
    assert_eq!(guest.call_read(&mut store, 7)?, -1);
    assert_eq!(guest.call_read(&mut store, 8)?, -1);

    // Consent is a TRUSTED HOST operation. The guest cannot grant itself.
    broker.grant_host_approved(instance, Capability::NotesRead, GrantScope::CurrentNote)?;
    assert_eq!(guest.call_read(&mut store, 7)?, i32::from(b'A'));
    assert_eq!(guest.call_read(&mut store, 8)?, -1);

    // Simulate the host UI selecting a different note without replacing the
    // guest. The stored CurrentNote authority must be re-evaluated live.
    store.data_mut().selected_note = Some(NoteId::new_for_host());
    assert_eq!(guest.call_read(&mut store, 7)?, -1);
    store.data_mut().selected_note = Some(id);
    assert_eq!(guest.call_read(&mut store, 7)?, i32::from(b'A'));

    // Revoke within the SAME running component instance: no stale lease.
    broker.revoke_all_for_host(instance)?;
    assert_eq!(guest.call_read(&mut store, 7)?, -1);
    assert_eq!(guest.call_read(&mut store, 8)?, -1);

    // This never writes to the note or accepts guest-controlled filesystem
    // arguments. In-flight revoke is separately proved in Core PLUGIN-04.
    assert_eq!(fs::read(&note_path)?, b"A confidential note");
    core.shutdown();
    Ok(())
}
