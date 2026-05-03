// tail_call_indirect_oob_trap.c
__attribute__((noinline)) static int dispatch_bad(int x) {
  typedef int (*unop_t)(int);
  unop_t fn = (unop_t)(unsigned int)12345;
  return fn(x);
}

int _start(void) { return dispatch_bad(7); }
