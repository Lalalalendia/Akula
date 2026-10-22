//! PLUGIN-05 resource lab: host-owned table and WIT resource semantics.
//! No Core, vault, filesystem, WASI, sockets, or user-visible plugin runtime.
use std::{error::Error, fs, path::PathBuf};
use wasmtime::{Config, Engine, Store};
use wasmtime::component::{bindgen, Component, HasSelf, Linker, Resource, ResourceTable};
use akula::plugin_resource::notes::{Host, HostNote};

type LabResult<T> = Result<T, Box<dyn Error>>;

bindgen!({
    world: "resource-lab",
    path: "../guest/wit",
    with: {
        "akula:plugin-resource/notes.note": BoundNote,
    },
    imports: { default: trappable },
});

/// The private resource table entry proves binding to a specific instance,
/// vault and grant generation; WIT itself conveys none of these identifiers.
pub struct BoundNote {
    instance: u64,
    vault: u64,
    epoch: u64,
    // Lab-only real plugin Core bridge; compiled out of release host.
    #[cfg(test)]
    actual: Option<CoreBoundNote>,
}

#[cfg(test)]
struct CoreBoundNote {
    instance: aquilum_core::plugins::PluginInstanceHandle,
    vault: aquilum_core::plugins::VaultId,
    note: aquilum_core::plugins::ScopedNoteHandle,
    // The broker's original revision must survive all later revoke/regrant.
    lease: aquilum_core::plugins::AuthorizationLease,
}

struct HostState {
    table: ResourceTable,
    instance: u64,
    vault: u64,
    epoch: u64,
    granted: bool,
    is_current: bool,
    drops: usize,
}

impl HostState {
    fn new() -> Self {
        Self {
            table: ResourceTable::new(),
            instance: 100,
            vault: 200,
            epoch: 1,
            granted: false,
            is_current: true,
            drops: 0,
        }
    }
}

impl Host for HostState {
    fn open_active(&mut self) -> wasmtime::Result<Option<Resource<BoundNote>>> {
        if !self.granted || !self.is_current {
            return Ok(None);
        }
        let note = self.table.push(BoundNote {
            instance: self.instance,
            vault: self.vault,
            epoch: self.epoch,
            #[cfg(test)]
            actual: None,
        })?;
        Ok(Some(note))
    }
}

impl HostNote for HostState {
    fn read_first(&mut self, note: Resource<BoundNote>) -> wasmtime::Result<i32> {
        // Borrow handles are valid only during the hostcall. Do not retain.
        let bound = self.table.get(&note)?;
        let current = self.granted
            && self.is_current
            && bound.instance == self.instance
            && bound.vault == self.vault
            && bound.epoch == self.epoch;
        Ok(if current { 65 } else { -1 })
    }

    fn drop(&mut self, note: Resource<BoundNote>) -> wasmtime::Result<()> {
        // Host table, not the guest, owns the underlying note identity.
        self.table.delete(note)?;
        self.drops += 1;
        Ok(())
    }
}

fn engine() -> LabResult<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    Ok(Engine::new(&config)?)
}

fn guest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../guest/target/wasm32-unknown-unknown/release/akula_plugin05_resource_guest.wasm")
}

fn component(engine: &Engine) -> LabResult<Component> {
    let guest = fs::read(guest_path())?;
    let bytes = wit_component::ComponentEncoder::default()
        .module(&guest)?
        .validate(true)
        .encode()?;
    Ok(Component::new(engine, bytes)?)
}

fn harness() -> LabResult<(Store<HostState>, ResourceLab)> {
    let engine = engine()?;
    let component = component(&engine)?;
    let mut store = Store::new(&engine, HostState::new());
    store.set_fuel(4_000_000)?;
    let mut linker = Linker::new(&engine);
    ResourceLab::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
    let guest = ResourceLab::instantiate(&mut store, &component, &linker)?;
    Ok((store, guest))
}

fn main() -> LabResult<()> {
    let (mut store, guest) = harness()?;
    assert!(!guest.call_acquire(&mut store)?);
    store.data_mut().granted = true;
    assert!(guest.call_acquire(&mut store)?);
    assert_eq!(guest.call_read_held(&mut store)?, 65);
    store.data_mut().epoch = 2;
    assert_eq!(guest.call_read_held(&mut store)?, -1);
    guest.call_release(&mut store)?;
    assert_eq!(store.data().drops, 1);
    println!("{{\"lab\":\"wit-resource\",\"revoke_after_acquire\":true,\"host_drops\":1}}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoke_and_epoch_invalidate_live_owned_resource() -> LabResult<()> {
        let (mut store, guest) = harness()?;
        assert!(!guest.call_acquire(&mut store)?);
        assert_eq!(guest.call_read_held(&mut store)?, -1);
        store.data_mut().granted = true;
        assert!(guest.call_acquire(&mut store)?);
        assert_eq!(guest.call_read_held(&mut store)?, 65);
        store.data_mut().granted = false;
        assert_eq!(guest.call_read_held(&mut store)?, -1);
        store.data_mut().granted = true;
        store.data_mut().epoch += 1;
        assert_eq!(guest.call_read_held(&mut store)?, -1);
        guest.call_release(&mut store)?;
        assert_eq!(store.data().drops, 1);
        assert!(guest.call_acquire(&mut store)?);
        assert_eq!(guest.call_read_held(&mut store)?, 65);
        guest.call_release(&mut store)?;
        assert_eq!(store.data().drops, 2);
        Ok(())
    }

    #[test]
    fn foreign_instance_vault_and_selection_fail_closed() -> LabResult<()> {
        let (mut store, guest) = harness()?;
        store.data_mut().granted = true;
        assert!(guest.call_acquire(&mut store)?);
        assert_eq!(guest.call_read_held(&mut store)?, 65);
        store.data_mut().instance += 1;
        assert_eq!(guest.call_read_held(&mut store)?, -1);
        store.data_mut().instance -= 1;
        store.data_mut().vault += 1;
        assert_eq!(guest.call_read_held(&mut store)?, -1);
        store.data_mut().vault -= 1;
        store.data_mut().is_current = false;
        assert_eq!(guest.call_read_held(&mut store)?, -1);
        guest.call_release(&mut store)?;
        assert_eq!(store.data().drops, 1);
        Ok(())
    }

    #[test]
    fn guest_cannot_instantiate_without_explicit_notes_import() -> LabResult<()> {
        let engine = engine()?;
        let component = component(&engine)?;
        let mut store = Store::new(&engine, HostState::new());
        let empty = Linker::<HostState>::new(&engine);
        assert!(ResourceLab::instantiate(&mut store, &component, &empty).is_err());
        Ok(())
    }

    #[test]
    fn repeated_acquire_release_reclaims_host_resource_entries() -> LabResult<()> {
        let (mut store, guest) = harness()?;
        store.data_mut().granted = true;
        for i in 1..=100 {
            assert!(guest.call_acquire(&mut store)?);
            assert_eq!(guest.call_read_held(&mut store)?, 65);
            guest.call_release(&mut store)?;
            assert_eq!(store.data().drops, i);
        }
        Ok(())
    }
}

#[cfg(test)]
mod actual_core_resource_tests;
