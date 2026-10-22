//! PLUGIN-05C: WIT/Component Model lab only; not linked into Akula.
use std::{error::Error, fs, path::PathBuf, time::Instant};
use wasmtime::{Config, Engine, Store};
use wasmtime::component::{bindgen, Component, HasSelf, Linker};

type LabResult<T> = Result<T, Box<dyn Error>>;

// Generate the actual host contract from the same WIT as the Rust guest.
bindgen!({
    world: "lab",
    path: "../guest/wit",
});

struct Host {
    granted: bool,
}

impl LabImports for Host {
    fn read_scoped(&mut self, token: u32) -> i32 {
        if self.granted && token == 7 {
            17
        } else {
            -1
        }
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
        .join("../guest/target/wasm32-unknown-unknown/release/akula_plugin05_component_guest.wasm")
}

fn compiled_component(engine: &Engine) -> LabResult<(Component, usize)> {
    let module = fs::read(guest_path())?;
    // No WASI adapter or external process: turn the wit-bindgen core wasm
    // + embedded WIT metadata into a fully typed Component in memory.
    let bytes = wit_component::ComponentEncoder::default()
        .module(&module)?
        .validate(true)
        .encode()?;
    let size = bytes.len();
    Ok((Component::new(engine, bytes)?, size))
}

fn make_linker(engine: &Engine) -> LabResult<Linker<Host>> {
    let mut linker = Linker::new(engine);
    Lab::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)?;
    Ok(linker)
}

fn linux_peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse::<u64>().ok())
    })
}

fn benchmark() -> LabResult<()> {
    let start = Instant::now();
    let engine = engine()?;
    let (component, component_size) = compiled_component(&engine)?;
    let encode_compile_us = start.elapsed().as_micros();
    let linker = make_linker(&engine)?;
    let mut store = Store::new(&engine, Host { granted: true });
    store.set_fuel(2_000_000)?;
    let start = Instant::now();
    let instance = Lab::instantiate(&mut store, &component, &linker)?;
    let instantiate_us = start.elapsed().as_micros();
    assert_eq!(instance.call_hello(&mut store)?, 42);
    assert_eq!(instance.call_read(&mut store, 8)?, -1);
    let mut calls = Vec::with_capacity(1_000);
    for _ in 0..1_000 {
        let start = Instant::now();
        assert_eq!(instance.call_read(&mut store, 7)?, 17);
        calls.push(start.elapsed().as_nanos());
    }
    calls.sort_unstable();
    let exe = fs::metadata(std::env::current_exe()?)?.len();
    let peak = linux_peak_rss_kib()
        .map(|n| n.to_string()).unwrap_or_else(|| "null".into());
    println!(
        "{{\"candidate\":\"wasmtime-component-wit\",\
        \"fixture\":\"rust-wit-scoped-note-v1\",\"host_version\":\"50.0.0\",\
        \"encode_compile_us\":{},\"instantiate_us\":{},\
        \"p50_call_ns\":{},\"p95_call_ns\":{},\"calls\":1000,\
        \"host_exe_bytes\":{},\"component_wasm_bytes\":{},\
        \"linux_peak_rss_kib\":{}}}",
        encode_compile_us, instantiate_us, calls[500], calls[950],
        exe, component_size, peak
    );
    Ok(())
}

fn main() -> LabResult<()> {
    benchmark()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wit_guest_never_gets_arbitrary_os_capabilities() -> LabResult<()> {
        let engine = engine()?;
        let (component, _) = compiled_component(&engine)?;
        // No WASI adapter, sockets, filesystem, stdout or process functions.
        // Instantiation must work with exactly one explicit WIT host import.
        let linker = make_linker(&engine)?;
        let mut store = Store::new(&engine, Host { granted: true });
        store.set_fuel(1_000_000)?;
        let guest = Lab::instantiate(&mut store, &component, &linker)?;
        assert_eq!(guest.call_hello(&mut store)?, 42);
        assert_eq!(guest.call_read(&mut store, 7)?, 17);
        assert_eq!(guest.call_read(&mut store, 8)?, -1);
        Ok(())
    }

    #[test]
    fn revoke_works_inside_same_live_component_instance() -> LabResult<()> {
        let engine = engine()?;
        let (component, _) = compiled_component(&engine)?;
        let linker = make_linker(&engine)?;
        let mut store = Store::new(&engine, Host { granted: true });
        store.set_fuel(1_000_000)?;
        let guest = Lab::instantiate(&mut store, &component, &linker)?;
        assert_eq!(guest.call_read(&mut store, 7)?, 17);
        store.data_mut().granted = false;
        assert_eq!(guest.call_read(&mut store, 7)?, -1);
        assert_eq!(guest.call_read(&mut store, 8)?, -1);
        Ok(())
    }

    #[test]
    fn no_access_if_never_granted() -> LabResult<()> {
        let engine = engine()?;
        let (component, _) = compiled_component(&engine)?;
        let linker = make_linker(&engine)?;
        let mut store = Store::new(&engine, Host { granted: false });
        store.set_fuel(1_000_000)?;
        let guest = Lab::instantiate(&mut store, &component, &linker)?;
        assert_eq!(guest.call_read(&mut store, 7)?, -1);
        Ok(())
    }
}

#[cfg(test)]
mod core_bridge_tests;
