;; memory_copy_backward_overlap.wat
;;
;; Overlap where src > dst (data shifts toward lower addresses). The
;; lowering's `need_backward = dst > src && (dst - src) < n` predicate is
;; FALSE here, so this exercises the word-copy + forward byte-tail path
;; on an overlapping range. Reading from higher addresses while writing
;; to lower ones is safe to do forward — the implementation must not
;; mistakenly switch to backward and clobber the (src - dst) gap.
;;
;; Pre-state: bytes 32..48 painted with byte (32+i) = 0x20 + i.
;; Call: memory.copy(dst=32, src=34, n=12) copies bytes 34..45 down to
;; bytes 32..43.
;;
;; Sentinels (top -> bottom):
;;   byte 32 -> 0x22  (copy low end == original byte 34)
;;   byte 43 -> 0x2D  (copy high end == original byte 45)
;;   byte 44 -> 0x2C  (just past the dst window, untouched)
;;   byte 47 -> 0x2F  (further untouched)
;; Result word = 0x222D2C2F.
(module
  (memory (export "memory") 1)
  (func $start (export "_start") (result i32)
    (i32.store (i32.const 32) (i32.const 0x23222120))
    (i32.store (i32.const 36) (i32.const 0x27262524))
    (i32.store (i32.const 40) (i32.const 0x2B2A2928))
    (i32.store (i32.const 44) (i32.const 0x2F2E2D2C))

    (memory.copy (i32.const 32) (i32.const 34) (i32.const 12))

    (i32.or
      (i32.shl (i32.load8_u (i32.const 32)) (i32.const 24))
      (i32.or
        (i32.shl (i32.load8_u (i32.const 43)) (i32.const 16))
        (i32.or
          (i32.shl (i32.load8_u (i32.const 44)) (i32.const 8))
          (i32.load8_u (i32.const 47)))))
  )
)
