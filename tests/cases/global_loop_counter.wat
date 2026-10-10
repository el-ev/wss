;; global_loop_counter.wat
;;
;; Mutable i32 globals as loop counter and accumulator, updated both in the
;; loop body and inside a callee; one immutable global as a multiplier.
;; acc: 1 -> 3 -> 10 -> 32 -> 99 -> 301 -> 908, i ends at 6.
;; Result = 908 ^ (6 << 16) = 0x0006038c.
(module
  (global $i (mut i32) (i32.const 0))
  (global $acc (mut i32) (i32.const 1))
  (global $k i32 (i32.const 3))
  (func $step
    (global.set $acc
      (i32.add (i32.mul (global.get $acc) (global.get $k)) (global.get $i))))
  (func $start (export "_start") (result i32)
    (loop $l
      (call $step)
      (global.set $i (i32.add (global.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (global.get $i) (i32.const 6))))
    (i32.xor (global.get $acc) (i32.shl (global.get $i) (i32.const 16))))
  (memory (export "memory") 1)
)
