;; exception_payload_recursive.wat
(module
  (tag $e (param i32))
  (func $rec (param $n i32)
    (if (i32.eqz (local.get $n))
      (then (throw $e (i32.const 42)))
      (else (call $rec (i32.sub (local.get $n) (i32.const 1))))))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $rec (i32.const 3))
        (i32.const 0))
      (catch $e))))
