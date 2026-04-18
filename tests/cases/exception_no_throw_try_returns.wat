(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (i32.const 5))
      (catch $e
        (i32.const 99)))))
