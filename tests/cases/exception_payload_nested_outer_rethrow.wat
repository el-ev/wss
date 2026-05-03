;; exception_payload_nested_outer_rethrow.wat
(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $e (i32.const 88))
            (i32.const 0))
          (catch $e
            drop
            rethrow 0)))
      (catch $e))))
