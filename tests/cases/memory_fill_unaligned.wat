;; memory_fill_unaligned.wat
;;
;; Exercises memory.fill cases the word-store + byte-tail lowering must
;; handle when the destination and/or length are not word-aligned:
;;
;;   (A) Unaligned dst, length > word: dst=5, n=10. Word loop performs
;;       two unaligned word stores (bytes 5..12), byte tail writes bytes
;;       13..14.
;;   (B) Aligned dst, sub-word length: dst=16, n=3. Word loop exits
;;       immediately, byte tail does all the work.
;;   (C) Pure byte path: dst=20, n=2. Even smaller than a word.
;;   (D) Zero-length fill must not touch any byte.
;;
;; Pre-state: bytes 0..32 painted with 0xAA so any stray writes show up.
;; Sentinels (top -> bottom of returned i32):
;;   byte  5 -> 0xC3  (A start, unaligned dst)
;;   byte 18 -> 0x9B  (B end, byte tail past aligned dst)
;;   byte 21 -> 0x4D  (C end, pure byte fill)
;;   byte 24 -> 0xAA  (D dst remains untouched on n=0)
;; Result word = 0xC39B4DAA.
(module
  (memory (export "memory") 1)
  (func $start (export "_start") (result i32)
    (i32.store (i32.const 0)  (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 4)  (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 8)  (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 12) (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 16) (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 20) (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 24) (i32.const 0xAAAAAAAA))
    (i32.store (i32.const 28) (i32.const 0xAAAAAAAA))

    ;; (A) Unaligned dst, length spanning multiple words.
    (memory.fill (i32.const 5)  (i32.const 0xC3) (i32.const 10))

    ;; (B) Aligned dst, sub-word length (byte-tail only).
    (memory.fill (i32.const 16) (i32.const 0x9B) (i32.const 3))

    ;; (C) Tiny fill: pure byte path.
    (memory.fill (i32.const 20) (i32.const 0x4D) (i32.const 2))

    ;; (D) Zero-length fill is a no-op.
    (memory.fill (i32.const 24) (i32.const 0xFF) (i32.const 0))

    (i32.or
      (i32.shl (i32.load8_u (i32.const 5))  (i32.const 24))
      (i32.or
        (i32.shl (i32.load8_u (i32.const 18)) (i32.const 16))
        (i32.or
          (i32.shl (i32.load8_u (i32.const 21)) (i32.const 8))
          (i32.load8_u (i32.const 24)))))
  )
)
