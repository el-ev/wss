source_filename = "if_else_nop_inlineasm"
target triple = "wasm32-unknown-unknown"

define i32 @_start() #0 {
entry:
  call void asm sideeffect "i32.const 1\0Aif void\0A  nop\0Aelse\0A  nop\0Aend_if", ""()
  ret i32 77
}

attributes #0 = { nounwind noinline optnone }
