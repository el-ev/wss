;; exception_nested_catch_all.wat
(module
  (tag $e (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $e)
            (i32.const 0))
          (catch_all
            (try (result i32)
              (do
                (throw $e)
                (i32.const 0))
              (catch $e
                (i32.const 36))))))
      (catch_all
        (i32.const 1)))))
