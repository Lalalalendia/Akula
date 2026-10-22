(module
  (import "wasi_snapshot_preview1" "sock_accept"
    (func $sock_accept (param i32 i32) (result i32)))
  (func (export "try_network") (result i32)
    i32.const 0
    i32.const 0
    call $sock_accept)
)
