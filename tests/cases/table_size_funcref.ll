; Call through a function table and read table.size 0.
; This returns (selected_call_result + table_size), where table_size is non-zero.
source_filename = "tests/cases/table_size_funcref.ll"
target datalayout = "e-m:e-p:32:32-p10:8:8-p20:8:8-i64:64-i128:128-n32:64-S128-ni:1:10:20"
target triple = "wasm32"

@seed = external global i32, align 4
@__indirect_function_table = external addrspace(1) global [0 x ptr addrspace(20)], align 1

define internal i32 @inc(i32 %x) {
entry:
  %y = add nsw i32 %x, 1
  ret i32 %y
}

define internal i32 @dec(i32 %x) {
entry:
  %y = add nsw i32 %x, -1
  ret i32 %y
}

define hidden i32 @_start() {
entry:
  %s = load volatile i32, ptr @seed, align 4
  %mask = and i32 %s, 1
  %is_zero = icmp eq i32 %mask, 0
  %f = select i1 %is_zero, ptr @dec, ptr @inc
  %v = call i32 %f(i32 41)
  %sz = call i32 @llvm.wasm.table.size(ptr addrspace(1) @__indirect_function_table)
  %out = add i32 %v, %sz
  ret i32 %out
}

declare i32 @llvm.wasm.table.size(ptr addrspace(1))
