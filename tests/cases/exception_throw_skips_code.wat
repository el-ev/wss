(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (local $x i32)
    (local.set $x (i32.const 3))
    (try
      (do
        (throw $e)
        (local.set $x (i32.const 999)))
      (catch $e
        (local.set $x (i32.add (local.get $x) (i32.const 10)))))
    (local.get $x)))
