;; memory_copy_basic.wat
;;
;; Exercises memory.copy in three regimes:
;;
;;   1. Non-overlapping copy: paint bytes 0..16 with a known pattern, copy
;;      them to bytes 64..80 (disjoint), then read back a sentinel byte.
;;   2. Forward-overlap copy (src < dst, src + n > dst): paint bytes 32..48
;;      with another pattern, then memory.copy(dst=34, src=32, n=12). This
;;      requires the backward walk to preserve the original src bytes.
;;   3. Zero-length copy is a no-op.
;;
;; Return word packs four sentinels:
;;   bits  0.. 7: byte 64  (should equal original byte 0 = 0x11)
;;   bits  8..15: byte 79  (should equal original byte 15 = 0xEF)
;;   bits 16..23: byte 34  (start of the shifted overlap region = original 32)
;;   bits 24..31: byte 45  (last byte written by the overlap copy = original 43)
;;
;; Original byte i in the patterns:
;;   bytes 0..16:  byte i = 0x11 + i  (so byte 0 = 0x11, byte 15 = 0x20)
;;   Wait — we need byte 15 = 0xEF. Let's pick the pattern carefully via a
;;   straight i32.store sequence so the asserts are obvious from the WAT.
(module
  (memory (export "memory") 1)
  (func $start (export "_start") (result i32)
    ;; Source pattern A at bytes 0..16 (4 i32 words, little-endian):
    ;;   bytes [0..4]   = 11 22 33 44
    ;;   bytes [4..8]   = 55 66 77 88
    ;;   bytes [8..12]  = 99 AA BB CC
    ;;   bytes [12..16] = DD 00 00 EF       ;; byte 15 = 0xEF (sentinel)
    (i32.store (i32.const 0)  (i32.const 0x44332211))
    (i32.store (i32.const 4)  (i32.const 0x88776655))
    (i32.store (i32.const 8)  (i32.const 0xCCBBAA99))
    (i32.store (i32.const 12) (i32.const 0xEF0000DD))

    ;; Source pattern B at bytes 32..48 (counting up from 0x20):
    ;;   byte (32+i) = 0x20 + i  for i in 0..16.
    (i32.store (i32.const 32) (i32.const 0x23222120))
    (i32.store (i32.const 36) (i32.const 0x27262524))
    (i32.store (i32.const 40) (i32.const 0x2B2A2928))
    (i32.store (i32.const 44) (i32.const 0x2F2E2D2C))

    ;; (1) Disjoint copy [0..16) -> [64..80).
    (memory.copy (i32.const 64) (i32.const 0) (i32.const 16))

    ;; (2) Forward-overlap: copy 12 bytes from 32 to 34 (regions overlap
    ;;     for 10 bytes). After the copy, byte 34..46 must equal the
    ;;     ORIGINAL bytes 32..44 (i.e., 0x20..0x2B). In particular,
    ;;     byte 34 = 0x20 and byte 45 = 0x2B.
    (memory.copy (i32.const 34) (i32.const 32) (i32.const 12))

    ;; (3) Zero-length copy must not touch any byte.
    (memory.copy (i32.const 100) (i32.const 0) (i32.const 0))

    ;; Pack sentinels.
    (i32.or
      (i32.load8_u (i32.const 64))            ;; expected 0x11
      (i32.or
        (i32.shl (i32.load8_u (i32.const 79)) (i32.const 8))  ;; 0xEF
        (i32.or
          (i32.shl (i32.load8_u (i32.const 34)) (i32.const 16)) ;; 0x20
          (i32.shl (i32.load8_u (i32.const 45)) (i32.const 24))) ;; 0x2B
      )
    )
  )
)
