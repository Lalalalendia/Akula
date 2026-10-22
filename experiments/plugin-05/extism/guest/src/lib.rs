//! 05B guest compiled for wasm32-unknown-unknown. No WASI interface.
use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    fn scoped_note(token: i32) -> i32;
}

#[plugin_fn]
pub fn hello(_input: String) -> FnResult<String> {
    Ok("42".to_owned())
}

#[plugin_fn]
pub fn read(token: String) -> FnResult<String> {
    let id = token.parse::<i32>()?;
    let response = unsafe { scoped_note(id)? };
    Ok(response.to_string())
}

#[plugin_fn]
pub fn spin(_input: String) -> FnResult<String> {
    loop {
        std::hint::spin_loop();
    }
}

#[plugin_fn]
pub fn grow(_input: String) -> FnResult<String> {
    // Make a bounded-memory failure observable, not a constant-folded Vec length.
    let mut huge = vec![7u8; 64 * 1024 * 1024];
    std::hint::black_box(&mut huge);
    Ok(huge.len().to_string())
}
