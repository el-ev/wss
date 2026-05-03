;; exception_local_try_catch.wat
(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $e)
        (i32.const 99))
      (catch $e
        (i32.const 42)))))
