;; i64_call_indirect.wat
(module
  (type $sig (func (param i64 i32) (result i64)))

  (func $adder (type $sig) (param i64 i32) (result i64)
    ;; return a + extend(b)
    local.get 0
    local.get 1
    i64.extend_i32_u
    i64.add
  )

  (table funcref (elem $adder))

  (func $start (export "_start") (result i64)
    i64.const 0xFEDCBA9800000000
    i32.const 0x12345678
    i32.const 0  ;; table index
    call_indirect (type $sig)
  )
  (memory (export "memory") 1)
)
