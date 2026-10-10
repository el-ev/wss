source_filename = "tail_call_mutual_even_odd.ll"
target triple = "wasm32-unknown-unknown"

@n_even = internal global i32 37, align 4
@n_even2 = internal global i32 40, align 4
@n_odd = internal global i32 9, align 4

define hidden i32 @is_even(i32 %n) #0 {
entry:
  %z = icmp eq i32 %n, 0
  br i1 %z, label %yes, label %rec
yes:
  ret i32 1
rec:
  %m = sub i32 %n, 1
  %r = musttail call i32 @is_odd(i32 %m)
  ret i32 %r
}

define hidden i32 @is_odd(i32 %n) #0 {
entry:
  %z = icmp eq i32 %n, 0
  br i1 %z, label %no, label %rec
no:
  ret i32 0
rec:
  %m = sub i32 %n, 1
  %r = musttail call i32 @is_even(i32 %m)
  ret i32 %r
}

define i32 @_start() #0 {
entry:
  %a = load volatile i32, ptr @n_even, align 4
  %b = load volatile i32, ptr @n_even2, align 4
  %c = load volatile i32, ptr @n_odd, align 4
  %ra = call i32 @is_even(i32 %a)
  %rb = call i32 @is_even(i32 %b)
  %rc = call i32 @is_odd(i32 %c)
  %s1 = shl i32 %ra, 4
  %s2 = shl i32 %rc, 1
  %o1 = or i32 %s1, %rb
  %o2 = or i32 %o1, %s2
  ret i32 %o2
}

attributes #0 = { nounwind noinline optnone "target-features"="+tail-call" }
