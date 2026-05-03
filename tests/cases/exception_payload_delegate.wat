;; exception_payload_delegate.wat
(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $e (i32.const 81))
            (i32.const 0))
          (delegate 0)))
      (catch $e))))
