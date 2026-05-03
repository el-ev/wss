;; exception_payload_call_chain.wat
(module
  (tag $e (param i32))
  (func $leaf
    (throw $e (i32.const 100)))
  (func $mid
    (call $leaf))
  (func $_start (export "_start") (result i32)
    (try (result i32)
      (do
        (call $mid)
        (i32.const 0))
      (catch $e))))
