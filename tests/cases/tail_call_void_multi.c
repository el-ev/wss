static int g_state = 3;

__attribute__((noinline)) static void add_bias(int x) { g_state += x + 1; }

__attribute__((noinline)) static void add_twice(int x) {
  add_bias(x * 2);
  return;
}

__attribute__((noinline)) static void add_chain(int x) {
  add_twice(x - 1);
  return;
}

int _start(void) {
  add_chain(6); // +11
  add_twice(3); // +7
  return g_state; // 21 (0x15)
}
