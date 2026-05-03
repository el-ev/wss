;; i64_select.wat
(module
  (func $start (export "_start") (result i64)
    i64.const 0x1111111122222222
    i64.const 0x3333333344444444
    i32.const 1
    select
    i64.const 0x1111111122222222
    i64.const 0x3333333344444444
    i32.const 0
    select
    i64.add
  )
  (memory (export "memory") 1)
)
