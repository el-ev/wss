(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $e)
            (i32.const 1))
          (delegate 0)))
      (catch $e
        (i32.const 21)))))
