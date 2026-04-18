(module
  (tag $a (param))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (try (result i32)
          (do
            (throw $a)
            (i32.const 1))
          (catch $a
            (i32.const 2))))
      (catch $a
        (i32.const 9)))))
