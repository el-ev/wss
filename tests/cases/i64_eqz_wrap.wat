(module
  (func $start (export "_start") (result i64)
    ;; i32.wrap_i64(0x0000000100000002) = 0x00000002
    ;; i64.extend_i32_s(0x00000002) = 0x0000000000000002
    i64.const 0x0000000100000002
    i32.wrap_i64
    i64.extend_i32_s

    ;; i64.eqz(0) = 1
    i64.const 0
    i64.eqz
    i64.extend_i32_u
    i64.add

    ;; i64.eqz(42) = 0
    i64.const 42
    i64.eqz
    i64.extend_i32_u
    i64.add

    ;; i64.extend8_s(0xFF) = -1
    i64.const 0xFF
    i64.extend8_s
    i64.add

    ;; i64.extend16_s(0x7FFF) = 32767
    i64.const 0x7FFF
    i64.extend16_s
    i64.add

    ;; i64.extend16_s(0x8000) = -32768
    i64.const 0x8000
    i64.extend16_s
    i64.add

    ;; add base
    i64.const 0xDEADBEEF00000000
    i64.add

    ;; i64.extend32_s(0x0000000080000000) = 0xFFFFFFFF80000000
    i64.const 0x0000000080000000
    i64.extend32_s
    i64.add
  )
  (memory (export "memory") 1)
)
