source_filename = "return_call_indirect"
target triple = "wasm32-unknown-unknown"

@seed = internal global i32 11, align 4
@fp = internal global ptr null, align 4

define hidden i32 @callee_mul3(i32 %x) #0 {
entry:
  %mul = mul i32 %x, 3
  ret i32 %mul
}

define hidden i32 @callee_add5(i32 %x) #0 {
entry:
  %add = add i32 %x, 5
  ret i32 %add
}

define hidden i32 @trampoline_ind(i32 %x) #1 {
entry:
  %f = load volatile ptr, ptr @fp, align 4
  %r = musttail call i32 %f(i32 %x)
  ret i32 %r
}

define i32 @_start() #1 {
entry:
  %v = load volatile i32, ptr @seed, align 4
  %is_odd = and i32 %v, 1
  %cond = icmp eq i32 %is_odd, 0
  br i1 %cond, label %choose_mul, label %choose_add

choose_mul:
  store volatile ptr @callee_mul3, ptr @fp, align 4
  br label %go

choose_add:
  store volatile ptr @callee_add5, ptr @fp, align 4
  br label %go

go:
  %u = add i32 %v, 1
  store volatile i32 %u, ptr @seed, align 4
  %r = call i32 @trampoline_ind(i32 %v)
  ret i32 %r
}

attributes #0 = { nounwind noinline optnone "target-features"="+tail-call" }
attributes #1 = { nounwind noinline optnone "target-features"="+tail-call" }
