// tail_call_void.c
static int done_flag = 0;

__attribute__((noinline)) static void write_value(int x) { done_flag = x + 11; }

__attribute__((noinline)) static void tail_store(int x) {
  write_value(x * 2);
  return;
}

int _start(void) {
  tail_store(14);
  return done_flag;
}
