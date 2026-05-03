;; exception_payload_catch_all.wat
(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $e (i32.const 123))
        (i32.const 0))
      (catch_all
        (i32.const 55)))))
