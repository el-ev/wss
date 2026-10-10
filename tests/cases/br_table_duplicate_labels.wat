;; br_table_duplicate_labels.wat
;;
;; br_table whose label vector repeats targets ($a $b $a $c $b, default $d).
;; pick: 0->10, 1->20, 2->10, 3->30, 4->20, 5->40, 6->40, -1 (huge u32)->40.
;; Result = 10 + 20*3 + 10*5 + 30*7 + 20*11 + 40*13 + 40*17 + 40*19 = 2510 = 0x9ce.
(module
  (memory (export "memory") 1)
  (func $pick (param $k i32) (result i32)
    (block $d
      (block $c
        (block $b
          (block $a
            (br_table $a $b $a $c $b $d (local.get $k)))
          (return (i32.const 10)))
        (return (i32.const 20)))
      (return (i32.const 30)))
    (i32.const 40))
  (func $start (export "_start") (result i32)
    (i32.add (call $pick (i32.const 0))
    (i32.add (i32.mul (call $pick (i32.const 1)) (i32.const 3))
    (i32.add (i32.mul (call $pick (i32.const 2)) (i32.const 5))
    (i32.add (i32.mul (call $pick (i32.const 3)) (i32.const 7))
    (i32.add (i32.mul (call $pick (i32.const 4)) (i32.const 11))
    (i32.add (i32.mul (call $pick (i32.const 5)) (i32.const 13))
    (i32.add (i32.mul (call $pick (i32.const 6)) (i32.const 17))
             (i32.mul (call $pick (i32.const -1)) (i32.const 19))))))))))
)
