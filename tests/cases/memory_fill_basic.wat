;; memory_fill_basic.wat
;;
;; Exercises memory.fill: pre-paint 16 bytes with 0xAA, then fill bytes 2..10
;; with 0x33, then read back four sentinel bytes packed into the i32 result.
;;
;; Layout after fill:
;;   byte 0  = 0xAA (untouched)
;;   byte 2  = 0x33 (start of fill region)
;;   byte 9  = 0x33 (last byte of fill region)
;;   byte 10 = 0xAA (just past the fill region)
;;
;; A trailing memory.fill with n=0 must be a no-op (byte 32 stays 0).
;; Result word = (0xAA<<24) | (0x33<<16) | (0x33<<8) | 0xAA = 0xAA3333AA.
(module
  (memory (export "memory") 1)
  (func $start (export "_start") (result i32)
    ;; Paint bytes 0..16 with 0xAA.
    (i32.store (i32.const 0)  (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 4)  (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 8)  (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 12) (i32.const 0xAAAAAAAA))

    ;; memory.fill(dst=2, val=0x33, n=8)
    (memory.fill (i32.const 2) (i32.const 0x33) (i32.const 8))

    ;; Zero-length fill must not write byte 32.
    (memory.fill (i32.const 32) (i32.const 0xFF) (i32.const 0))

    ;; Pack four sentinel bytes into the return value.
    (i32.or
      (i32.shl (i32.load8_u (i32.const 0))  (i32.const 24))
      (i32.or
        (i32.shl (i32.load8_u (i32.const 2))  (i32.const 16))
        (i32.or
          (i32.shl (i32.load8_u (i32.const 9))  (i32.const 8))
          (i32.load8_u (i32.const 10)))))
  )
)
