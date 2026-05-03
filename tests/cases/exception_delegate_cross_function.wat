;; exception_delegate_cross_function.wat
(module
  (tag $e (param))
  (func $inner
    (try
      (do
        (throw $e))
      (delegate 0)))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $inner)
        (i32.const 1))
      (catch $e
        (i32.const 65)))))
