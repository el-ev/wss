;; memory_copy_forward_overlap.wat
;;
;; Overlapping memory.copy with dst < src (dst=2, src=5, n=8), the mirror of
;; memory_copy_backward_overlap. A forward byte-by-byte copy is correct here;
;; a backward one would read already-overwritten bytes.
;;
;; Pre-state bytes 0..16: 11 22 33 44 55 66 77 88 99 AA BB CC DD EE FF 10
;; Post-state bytes 2..9 = original bytes 5..12 = 66 77 88 99 AA BB CC DD
;; Sentinels: byte 1 = 22 (untouched), byte 2 = 66, byte 9 = DD, byte 10 = BB.
;; Result word = 0x2266DDBB.
(module
  (memory (export "memory") 1)
  (func $start (export "_start") (result i32)
    (i32.store (i32.const 0)  (i32.const 0x44332211))
    (i32.store (i32.const 4)  (i32.const 0x88776655))
    (i32.store (i32.const 8)  (i32.const 0xCCBBAA99))
    (i32.store (i32.const 12) (i32.const 0x10FFEEDD))

    (memory.copy (i32.const 2) (i32.const 5) (i32.const 8))

    (i32.or
      (i32.shl (i32.load8_u (i32.const 1)) (i32.const 24))
      (i32.or
        (i32.shl (i32.load8_u (i32.const 2)) (i32.const 16))
        (i32.or
          (i32.shl (i32.load8_u (i32.const 9)) (i32.const 8))
          (i32.load8_u (i32.const 10)))))
  )
)
