;; exception_outer_catches_inner_rethrow.wat
(module
  (tag $a (param))
  (tag $b (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $a)
            (i32.const 0))
          (catch $a
            (rethrow 0)
            (i32.const 0))))
      (catch $a
        (i32.const 47))
      (catch $b
        (i32.const 48)))))
