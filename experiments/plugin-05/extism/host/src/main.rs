//! PLUGIN-05B research-only Extism Rust host. Not linked into Akula Core.
use extism::{host_fn, Manifest, Plugin, PluginBuilder, UserData, Wasm, PTR};
use std::{
    error::Error,
    fs,
    path::PathBuf,
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    time::{Duration, Instant},
};

type LabResult<T> = Result<T, Box<dyn Error>>;
const NETWORK_WAT: &str = include_str!("../../../wasmtime/fixtures/network.wat");
const BENCH_CALLS: usize = 1000;

host_fn!(scoped_note(user_data: Arc<AtomicBool>; token: i32) -> i32 {
    let handle = user_data.get()?;
    let granted = handle.lock().unwrap();
    Ok(if granted.load(Ordering::Acquire) && token == 7 { 17 } else { -1 })
});

fn guest_bytes() -> LabResult<Vec<u8>> {
    let module = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../guest/target/wasm32-unknown-unknown/release/akula_plugin05_extism_guest.wasm");
    Ok(fs::read(module)?)
}

fn new_plugin_with_grant(guest: &[u8], grant: Arc<AtomicBool>, fuel: u64) -> LabResult<Plugin> {
    let manifest = Manifest::new([Wasm::data(guest.to_vec())])
        .disallow_all_hosts()
        .with_memory_max(128)
        .with_timeout(Duration::from_millis(500));
    Ok(PluginBuilder::new(manifest)
        .with_wasi(false)
        .with_fuel_limit(fuel)
        .with_function(
            "scoped_note",
            [PTR],
            [PTR],
            UserData::new(grant),
            scoped_note,
        )
        .build()?)
}

fn new_plugin(guest: &[u8], allowed: bool, fuel: u64) -> LabResult<Plugin> {
    new_plugin_with_grant(guest, Arc::new(AtomicBool::new(allowed)), fuel)
}

fn linux_peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<u64>().ok())
    })
}

fn benchmark() -> LabResult<()> {
    let guest = guest_bytes()?;
    let started = Instant::now();
    let mut plugin = new_plugin(&guest, true, 1_000_000_000)?;
    let plugin_compile_us = started.elapsed().as_micros();
    assert_eq!(plugin.call::<&str, &str>("hello", "")?, "42");
    assert_eq!(plugin.call::<&str, &str>("read", "8")?, "-1");

    let mut samples = Vec::with_capacity(BENCH_CALLS);
    for _ in 0..BENCH_CALLS {
        let started = Instant::now();
        assert_eq!(plugin.call::<&str, &str>("read", "7")?, "17");
        samples.push(started.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let bytes = fs::metadata(std::env::current_exe()?)?.len();
    let guest_bytes = guest.len();
    let rss = linux_peak_rss_kib()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".into());
    println!(
        "{{\"candidate\":\"extism\",\"version\":\"1.41.0\",\
         \"fixture\":\"rust-pdk-scoped-note-v1\",\"plugin_compile_us\":{},\
         \"p50_call_ns\":{},\"p95_call_ns\":{},\"samples\":{},\
         \"standalone_exe_bytes\":{},\"guest_wasm_bytes\":{},\
         \"linux_peak_rss_kib\":{}}}",
        plugin_compile_us, samples[500], samples[950], BENCH_CALLS, bytes, guest_bytes, rss,
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
    fn hello_and_scoped_read_require_explicit_synthetic_token() -> LabResult<()> {
        let guest = guest_bytes()?;
        for allowed in [false, true] {
            let mut plugin = new_plugin(&guest, allowed, 20_000_000)?;
            assert_eq!(plugin.call::<&str, &str>("hello", "")?, "42");
            assert_eq!(plugin.call::<&str, &str>("read", "8")?, "-1");
            assert_eq!(
                plugin.call::<&str, &str>("read", "7")?,
                if allowed { "17" } else { "-1" },
            );
        }
        Ok(())
    }

    #[test]
    fn revoke_prevents_next_hostcall_in_same_running_instance() -> LabResult<()> {
        let guest = guest_bytes()?;
        let granted = Arc::new(AtomicBool::new(true));
        let mut plugin = new_plugin_with_grant(&guest, Arc::clone(&granted), 20_000_000)?;
        assert_eq!(plugin.call::<&str, &str>("read", "7")?, "17");
        granted.store(false, Ordering::Release);
        assert_eq!(plugin.call::<&str, &str>("read", "7")?, "-1");
        assert_eq!(plugin.call::<&str, &str>("read", "8")?, "-1");
        Ok(())
    }

    #[test]
    fn undeclared_wasi_socket_import_is_not_resolved() -> LabResult<()> {
        let bytes = wat::parse_str(NETWORK_WAT)?;
        let manifest = Manifest::new([Wasm::data(bytes)]).disallow_all_hosts();
        assert!(
            PluginBuilder::new(manifest).with_wasi(false).build().is_err(),
            "network import must not resolve without explicitly granted WASI"
        );
        Ok(())
    }

    #[test]
    fn untrusted_guest_loop_is_interrupted() -> LabResult<()> {
        let guest = guest_bytes()?;
        let mut plugin = new_plugin(&guest, false, 500_000)?;
        let error = plugin.call::<&str, &str>("spin", "").expect_err("must trap");
        eprintln!("PLUGIN05B_LOOP_TRAP: {error:#}");
        Ok(())
    }

    #[test]
    fn guest_growth_beyond_memory_max_is_rejected() -> LabResult<()> {
        let guest = guest_bytes()?;
        let mut plugin = new_plugin(&guest, false, 30_000_000)?;
        let error = plugin.call::<&str, &str>("grow", "").expect_err("must reject growth");
        eprintln!("PLUGIN05B_GROW_TRAP: {error:#}");
        Ok(())
    }
}
