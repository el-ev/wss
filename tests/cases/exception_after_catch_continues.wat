(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (local $acc i32)
    (local.set $acc (i32.const 100))
    (try
      (do
        (throw $e))
      (catch $e
        (local.set $acc (i32.const 200))))
    (local.set $acc (i32.add (local.get $acc) (i32.const 50)))
    (local.get $acc)))
