(module
  (memory (export "memory") 1)
  (func (export "grow") (result i32)
    i32.const 100
    memory.grow)
)
