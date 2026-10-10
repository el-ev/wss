;; exception_loop_catch_accumulate.wat
;;
;; A try inside a loop body: odd iterations throw i*10 and the catch adds 1,
;; even iterations fall through with i. Sum over i = 0..6:
;; 0 + 11 + 2 + 31 + 4 + 51 + 6 = 105 = 0x69.
(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (local $i i32)
    (local $acc i32)
    (loop $l
      (local.set $acc
        (i32.add (local.get $acc)
          (try (result i32)
            (do
              (if (i32.and (local.get $i) (i32.const 1))
                (then (throw $e (i32.mul (local.get $i) (i32.const 10)))))
              (local.get $i))
            (catch $e
              (i32.add (i32.const 1))))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 7))))
    (local.get $acc))
  (memory (export "memory") 1)
)
