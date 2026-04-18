(module
  (tag $e (param i32))
  (func $thrower
    (throw $e (i32.const 200)))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $thrower)
        (i32.const 1))
      (catch $e))))
