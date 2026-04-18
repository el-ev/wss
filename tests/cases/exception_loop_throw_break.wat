(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (local $i i32)
    (local $sum i32)
    (try (result i32)
      (do
        (loop $l
          (local.set $sum (i32.add (local.get $sum) (local.get $i)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (if (i32.eq (local.get $i) (i32.const 5))
            (then (throw $e)))
          (br $l))
        (i32.const -1))
      (catch $e
        (local.get $sum)))))
