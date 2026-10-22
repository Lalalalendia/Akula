//! Real WIT guest, compiled to wasm32-unknown-unknown, with no WASI imports.
wit_bindgen::generate!();

struct GuestImpl;

impl Guest for GuestImpl {
    fn hello() -> u32 {
        42
    }

    fn read(token: u32) -> i32 {
        read_scoped(token)
    }
}

export!(GuestImpl);
