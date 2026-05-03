;; exception_payload_zero.wat
(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $e (i32.const 0))
        (i32.const 999))
      (catch $e
        i32.const 3
        i32.add))))
