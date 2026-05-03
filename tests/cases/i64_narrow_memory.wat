;; i64_narrow_memory.wat
(module
  (func $start (export "_start") (result i64)
    ;; Store 0xFEDCBA9876543210 at offset 0
    i32.const 0
    i64.const 0xFEDCBA9876543210
    i64.store

    ;; i64.load8_u from offset 0: 0x10
    i32.const 0
    i64.load8_u

    ;; i64.load8_s from offset 7: 0xFE sign-extended = 0xFFFFFFFFFFFFFFFE
    i32.const 7
    i64.load8_s
    i64.add

    ;; i64.load16_u from offset 0: 0x3210
    i32.const 0
    i64.load16_u
    i64.add

    ;; i64.load16_s from offset 6: 0xFEDC sign-extended = 0xFFFFFFFFFFFFFEDC
    i32.const 6
    i64.load16_s
    i64.add

    ;; i64.store8 at offset 16
    i32.const 16
    i64.const 0xAB
    i64.store8

    ;; i64.store16 at offset 17
    i32.const 17
    i64.const 0xCDEF
    i64.store16

    ;; i64.store32 at offset 19
    i32.const 19
    i64.const 0x12345678
    i64.store32

    ;; Load back i64 from offset 16
    ;; Bytes: [AB, EF, CD, 78, 56, 34, 12, 00]
    ;; LE i64: 0x0012345678CDEFAB
    i32.const 16
    i64.load
    i64.add
  )
  (memory (export "memory") 1)
)
