(module
  (func (export "spin") (result i32)
    (loop $forever
      br $forever)
    i32.const 0)
)
