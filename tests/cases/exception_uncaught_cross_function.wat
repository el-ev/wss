(module
  (tag $e (param))
  (func $throwee
    (throw $e))
  (func $_start (export "_start") (result i32)
    (call $throwee)
    (i32.const 0)))
