//! PLUGIN-05A isolated Wasmtime experiment. NEVER linked into Akula.
use std::{error::Error, fs, time::Instant};
use wasmtime::{
    Caller, Config, Engine, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

const HOSTCALL_WAT: &str = include_str!("../fixtures/hostcall.wat");
const NETWORK_WAT: &str = include_str!("../fixtures/network.wat");
const LOOP_WAT: &str = include_str!("../fixtures/loop.wat");
const MEMORY_WAT: &str = include_str!("../fixtures/memory.wat");
const MAX_GUEST_MEMORY: usize = 1024 * 1024;
type LabResult<T> = Result<T, Box<dyn Error>>;

struct Host {
    granted: bool,
    limits: StoreLimits,
}

fn new_engine() -> LabResult<Engine> {
    let mut config = Config::new();
    config.consume_fuel(true);
    Ok(Engine::new(&config)?)
}

fn new_store(engine: &Engine, granted: bool, fuel: u64) -> LabResult<Store<Host>> {
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_GUEST_MEMORY)
        .memories(1)
        .tables(0)
        .instances(1)
        .build();
    let mut store = Store::new(engine, Host { granted, limits });
    store.limiter(|host| &mut host.limits);
    store.set_fuel(fuel)?;
    Ok(store)
}

fn new_linker(engine: &Engine) -> LabResult<Linker<Host>> {
    let mut linker = Linker::new(engine);
    // This is a synthetic token oracle. No Core, fs, WASI or sockets exist here.
    linker.func_wrap(
        "akula",
        "scoped_note",
        |caller: Caller<'_, Host>, token: i32| -> i32 {
            if caller.data().granted && token == 7 {
                17
            } else {
                -1
            }
        },
    )?;
    Ok(linker)
}

fn instantiate(
    linker: &Linker<Host>,
    store: &mut Store<Host>,
    module: &Module,
) -> LabResult<Instance> {
    Ok(linker.instantiate(store, module)?)
}

fn peak_rss_kib_linux() -> Option<u64> {
    let contents = fs::read_to_string("/proc/self/status").ok()?;
    contents.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
    })
}

fn run_benchmark() -> LabResult<()> {
    let compile_start = Instant::now();
    let engine = new_engine()?;
    let module = Module::new(&engine, HOSTCALL_WAT)?;
    let engine_and_compile_us = compile_start.elapsed().as_micros();
    let linker = new_linker(&engine)?;
    let mut store = new_store(&engine, true, 2_000_000)?;
    let start = Instant::now();
    let instance = instantiate(&linker, &mut store, &module)?;
    let instantiate_us = start.elapsed().as_micros();
    let hello = instance.get_typed_func::<(), i32>(&mut store, "hello")?;
    assert_eq!(hello.call(&mut store, ())?, 42);
    let read = instance.get_typed_func::<i32, i32>(&mut store, "read")?;
    assert_eq!(read.call(&mut store, 8)?, -1);

    let mut samples = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = Instant::now();
        assert_eq!(read.call(&mut store, 7)?, 17);
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p95 = samples[(samples.len() * 95) / 100];
    let bytes = fs::metadata(std::env::current_exe()?)?.len();
    let peak_rss = peak_rss_kib_linux()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".into());
    println!(
        "{{\"candidate\":\"wasmtime\",\"version\":\"50.0.0\",\"fixture\":\"scoped-note-v1\",\
         \"engine_compile_us\":{},\"instantiate_us\":{},\"p50_call_ns\":{},\
         \"p95_call_ns\":{},\"samples\":1000,\"standalone_exe_bytes\":{},\
         \"linux_peak_rss_kib\":{}}}",
        engine_and_compile_us, instantiate_us, p50, p95, bytes, peak_rss,
    );
    Ok(())
}

fn main() -> LabResult<()> {
    run_benchmark()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FUEL: u64 = 30_000;

    #[test]
    fn hello_and_positive_negative_synthetic_read() -> LabResult<()> {
        let engine = new_engine()?;
        let module = Module::new(&engine, HOSTCALL_WAT)?;
        let linker = new_linker(&engine)?;
        for granted in [false, true] {
            let mut store = new_store(&engine, granted, FUEL)?;
            let instance = instantiate(&linker, &mut store, &module)?;
            let hello = instance.get_typed_func::<(), i32>(&mut store, "hello")?;
            assert_eq!(hello.call(&mut store, ())?, 42);
            let read = instance.get_typed_func::<i32, i32>(&mut store, "read")?;
            assert_eq!(read.call(&mut store, 8)?, -1);
            assert_eq!(read.call(&mut store, 7)?, if granted { 17 } else { -1 });
        }
        Ok(())
    }

    #[test]
    fn revoke_prevents_next_hostcall_in_same_running_instance() -> LabResult<()> {
        let engine = new_engine()?;
        let module = Module::new(&engine, HOSTCALL_WAT)?;
        let mut store = new_store(&engine, true, FUEL)?;
        let instance = instantiate(&new_linker(&engine)?, &mut store, &module)?;
        let read = instance.get_typed_func::<i32, i32>(&mut store, "read")?;
        assert_eq!(read.call(&mut store, 7)?, 17);
        store.data_mut().granted = false;
        assert_eq!(read.call(&mut store, 7)?, -1);
        assert_eq!(read.call(&mut store, 8)?, -1);
        Ok(())
    }

    #[test]
    fn rejects_undeclared_wasi_network_capability() -> LabResult<()> {
        let engine = new_engine()?;
        let module = Module::new(&engine, NETWORK_WAT)?;
        let mut store = new_store(&engine, true, FUEL)?;
        assert!(
            new_linker(&engine)?
                .instantiate(&mut store, &module)
                .is_err(),
            "no WASI/socket import may be dynamically resolved"
        );
        Ok(())
    }

    #[test]
    fn infinite_guest_traps_when_fuel_exhausted() -> LabResult<()> {
        let engine = new_engine()?;
        let module = Module::new(&engine, LOOP_WAT)?;
        let mut store = new_store(&engine, false, FUEL)?;
        let instance = instantiate(&new_linker(&engine)?, &mut store, &module)?;
        let spin = instance.get_typed_func::<(), i32>(&mut store, "spin")?;
        assert!(spin.call(&mut store, ()).is_err(), "guest must trap");
        Ok(())
    }

    #[test]
    fn guest_cannot_grow_memory_past_configured_limit() -> LabResult<()> {
        let engine = new_engine()?;
        let module = Module::new(&engine, MEMORY_WAT)?;
        let mut store = new_store(&engine, false, FUEL)?;
        let instance = instantiate(&new_linker(&engine)?, &mut store, &module)?;
        let grow = instance.get_typed_func::<(), i32>(&mut store, "grow")?;
        assert_eq!(grow.call(&mut store, ())?, -1);
        Ok(())
    }
}
