(module
  (tag $e (param i32))
  (func $_start (export "_start") (result i32)
    (local $i i32)
    (try (result i32)
      (do
        (block $exit
          (loop $top
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br_if $exit (i32.ge_s (local.get $i) (i32.const 3)))
            (br $top)))
        (throw $e (i32.const 50))
        (i32.const 0))
      (catch $e
        local.get $i
        i32.add))))
