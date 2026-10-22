(module
  (import "akula" "scoped_note" (func $scoped_note (param i32) (result i32)))
  (func (export "hello") (result i32)
    i32.const 42)
  (func (export "read") (param i32) (result i32)
    local.get 0
    call $scoped_note)
)
