;; i64_globals.wat
(module
  (global $g0 (mut i64) (i64.const 0))
  (global $g1 (mut i64) (i64.const 0xDEADBEEF12345678))

  (func $start (export "_start") (result i64)
    ;; write a value to g0
    i64.const 0x00000001FFFFFFFE
    global.set $g0

    ;; read g1 (initialized to 0xDEADBEEF12345678)
    global.get $g1

    ;; read g0 back and add
    global.get $g0
    i64.add
  )
  (memory (export "memory") 1)
)
