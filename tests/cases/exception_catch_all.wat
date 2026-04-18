(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $e)
        (i32.const 11))
      (catch_all
        (i32.const 7)))))
