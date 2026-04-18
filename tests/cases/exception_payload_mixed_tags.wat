(module
  (tag $a (param))
  (tag $b (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $b (i32.const 40))
        (i32.const 0))
      (catch $a
        (i32.const 1))
      (catch $b
        i32.const 2
        i32.add))))
