;; exception_catch_second_of_two.wat
(module
  (tag $a (param))
  (tag $b (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (throw $b)
        (i32.const 1))
      (catch $a
        (i32.const 31))
      (catch $b
        (i32.const 44)))))
