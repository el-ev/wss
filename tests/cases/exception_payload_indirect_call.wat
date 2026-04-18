(module
  (type $void (func))
  (tag $e (param i32))
  (table 1 funcref)
  (elem (i32.const 0) $thrower)
  (func $thrower
    (throw $e (i32.const 77)))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call_indirect (type $void) (i32.const 0))
        (i32.const 0))
      (catch $e))))
