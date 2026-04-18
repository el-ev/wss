(module
  (tag $e (param))
  (func $throwee
    (throw $e))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $throwee)
        (i32.const 55))
      (catch $e
        (i32.const 17)))))
