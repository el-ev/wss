;; memory_fill_zero_len.wat
;;
;; Zero-length memory.fill and memory.copy must not touch memory at all,
;; and a one-byte fill must touch exactly one byte.
;;
;; Pre-state: bytes 0..8 = 0x5A.
;; memory.fill(4, 0x33, 0)  -> no change
;; memory.copy(0, 4, 0)     -> no change
;; memory.fill(5, 0x77, 1)  -> byte 5 = 0x77
;; Result = i32.load(4) = bytes 4..7 = 5A 77 5A 5A -> 0x5A5A775A.
(module
  (memory (export "memory") 1)
  (func $start (export "_start") (result i32)
    (i32.store (i32.const 0) (i32.const 0x5A5A5A5A))
    (i32.store (i32.const 4) (i32.const 0x5A5A5A5A))
    (memory.fill (i32.const 4) (i32.const 0x33) (i32.const 0))
    (memory.copy (i32.const 0) (i32.const 4) (i32.const 0))
    (memory.fill (i32.const 5) (i32.const 0x77) (i32.const 1))
    (i32.load (i32.const 4))
  )
)
