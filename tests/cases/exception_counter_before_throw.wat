(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (local $n i32)
    (try (result i32)
      (do
        (local.set $n (i32.add (local.get $n) (i32.const 3)))
        (local.set $n (i32.add (local.get $n) (i32.const 4)))
        (throw $e)
        (i32.const 0))
      (catch $e
        (local.get $n)))))
