;; exception_catch_all_then_return.wat
(module
  (tag $e (param))
  (func $thrower
    (throw $e))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $thrower)
        (i32.const 1))
      (catch_all
        (i32.const 23)))))
