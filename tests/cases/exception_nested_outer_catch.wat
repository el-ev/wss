;; exception_nested_outer_catch.wat
(module
  (tag $a (param))
  (tag $b (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $b)
            (i32.const 1))
          (catch $a
            (i32.const 2))))
      (catch $b
        (i32.const 8)))))
