;; memory_copy_unaligned.wat
;;
;; Disjoint memory.copy with unaligned src, unaligned dst, and a length
;; that is not a multiple of four. Forces the word loop to perform
;; unaligned loads/stores and the forward byte tail to drain the
;; remainder.
;;
;; Pre-state:
;;   bytes 0..16 = byte i takes the pattern below (0x11,0x22,0x33,...)
;;   bytes 60..80 = 0x5A (sentinel canary so stray writes are visible).
;;
;; Call: memory.copy(dst=65, src=3, n=9). Both endpoints unaligned and
;; n=9 = 2 words + 1 byte. dst > src but (dst - src) = 62 >> n = 9 so
;; the lowering takes the forward word + byte-tail path.
;;
;; Expected post-state:
;;   bytes 65..73 = original bytes 3..11
;;     byte 65 = 0x44  (orig byte 3)
;;     byte 73 = 0xCC  (orig byte 11)
;;   bytes 64 and 74 remain 0x5A (canary intact on both sides).
;;
;; Sentinels (top -> bottom):
;;   byte 64 -> 0x5A
;;   byte 65 -> 0x44
;;   byte 73 -> 0xCC
;;   byte 74 -> 0x5A
;; Result word = 0x5A44CC5A.
(module
  (memory (export "memory") 1)
  (func $start (export "_start") (result i32)
    ;; Source pattern at bytes 0..16 (little-endian word view):
    ;;   bytes [0..4]   = 11 22 33 44
    ;;   bytes [4..8]   = 55 66 77 88
    ;;   bytes [8..12]  = 99 AA BB CC
    ;;   bytes [12..16] = DD EE FF 10
    (i32.store (i32.const 0)  (i32.const 0x44332211))
    (i32.store (i32.const 4)  (i32.const 0x88776655))
    (i32.store (i32.const 8)  (i32.const 0xCCBBAA99))
    (i32.store (i32.const 12) (i32.const 0x10FFEEDD))

    ;; Paint the destination window with 0x5A canary.
    (i32.store (i32.const 60) (i32.const 0x5A5A5A5A))
    (i32.store (i32.const 64) (i32.const 0x5A5A5A5A))
    (i32.store (i32.const 68) (i32.const 0x5A5A5A5A))
    (i32.store (i32.const 72) (i32.const 0x5A5A5A5A))
    (i32.store (i32.const 76) (i32.const 0x5A5A5A5A))

    (memory.copy (i32.const 65) (i32.const 3) (i32.const 9))

    (i32.or
      (i32.shl (i32.load8_u (i32.const 64)) (i32.const 24))
      (i32.or
        (i32.shl (i32.load8_u (i32.const 65)) (i32.const 16))
        (i32.or
          (i32.shl (i32.load8_u (i32.const 73)) (i32.const 8))
          (i32.load8_u (i32.const 74)))))
  )
)
