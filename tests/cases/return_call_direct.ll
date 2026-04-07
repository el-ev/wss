source_filename = "return_call_direct"
target triple = "wasm32-unknown-unknown"

@seed = internal global i32 9, align 4

define hidden i32 @callee(i32 %x) #0 {
entry:
  %add = add i32 %x, 7
  ret i32 %add
}

define hidden i32 @trampoline(i32 %x) #1 {
entry:
  %r = musttail call i32 @callee(i32 %x)
  ret i32 %r
}

define i32 @_start() #1 {
entry:
  %v = load volatile i32, ptr @seed, align 4
  %u = xor i32 %v, 12345
  store volatile i32 %u, ptr @seed, align 4
  %r = call i32 @trampoline(i32 %v)
  ret i32 %r
}

attributes #0 = { nounwind noinline optnone "target-features"="+tail-call" }
attributes #1 = { nounwind noinline optnone "target-features"="+tail-call" }
