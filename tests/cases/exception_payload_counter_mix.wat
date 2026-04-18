(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (local $n i32)
    (try (result i32)
      (do
        (local.set $n (i32.const 5))
        (throw $e (i32.const 11))
        (i32.const 0))
      (catch $e
        local.get $n
        i32.add))))
