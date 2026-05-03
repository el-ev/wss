;; exception_indirect_call_catch.wat
(module
  (type $void (func))
  (tag $e (param))
  (table 1 funcref)
  (elem (i32.const 0) $thrower)
  (func $thrower
    (throw $e))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call_indirect (type $void) (i32.const 0))
        (i32.const 1))
      (catch $e
        (i32.const 91)))))
