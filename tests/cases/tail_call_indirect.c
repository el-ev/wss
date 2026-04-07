typedef int (*binop_t)(int a, int b);

static volatile int g_idx = 2;

__attribute__((noinline)) static int add(int a, int b) { return a + b; }
__attribute__((noinline)) static int sub(int a, int b) { return a - b; }
__attribute__((noinline)) static int mul(int a, int b) { return a * b; }

__attribute__((noinline)) static int run_table(int idx, int a, int b) {
  binop_t table[3] = {add, sub, mul};
  return table[idx](a, b);
}

int _start(void) {
  int idx = g_idx;
  return run_table(idx, 6, 7); // 42 (0x2a)
}
