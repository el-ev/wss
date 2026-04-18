(module
  (tag $e (param))
  (func $helper
    (try
      (do
        (throw $e))
      (catch $e
        (rethrow 0))))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $helper)
        (i32.const 1))
      (catch $e
        (i32.const 88)))))
