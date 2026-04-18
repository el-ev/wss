(module
  (tag $e (param))
  (func $leaf (throw $e))
  (func $mid (call $leaf))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $mid)
        (i32.const 5))
      (catch $e
        (i32.const 77)))))
