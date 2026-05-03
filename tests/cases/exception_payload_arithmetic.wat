;; exception_payload_arithmetic.wat
(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $e (i32.const 10))
        (i32.const 0))
      (catch $e
        i32.const 5
        i32.add))))
