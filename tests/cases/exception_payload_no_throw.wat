(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (i32.const 17))
      (catch $e))))
