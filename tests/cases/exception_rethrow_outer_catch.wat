(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $e)
            (i32.const 1))
          (catch $e
            (rethrow 0)
            (i32.const 2))))
      (catch $e
        (i32.const 13)))))
