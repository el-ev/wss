;; i64_spill_call.wat
(module
  (func $helper (param i32) (result i64)
    ;; return 0x0000000100000000 * param (shifts are unsupported, use add loop)
    (local i64)
    (local.set 1 (i64.const 0x0000000100000000))
    (if (i32.eqz (local.get 0))
      (then (return (i64.const 0)))
    )
    local.get 1
  )

  (func $start (export "_start") (result i64)
    (local $a i64)
    (local $b i64)
    ;; set up two i64 locals that must survive across calls
    (local.set $a (i64.const 0xAAAAAAAA00000001))
    (local.set $b (i64.const 0x0000000011111111))

    ;; call helper — $a and $b must be spilled and restored
    (drop (call $helper (i32.const 1)))

    ;; use both locals after the call
    (i64.add (local.get $a) (local.get $b))
  )
  (memory (export "memory") 1)
)
