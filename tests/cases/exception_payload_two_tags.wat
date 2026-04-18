(module
  (tag $a (param i32))
  (tag $b (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $b (i32.const 66))
        (i32.const 0))
      (catch $a
        drop
        i32.const 1)
      (catch $b))))
