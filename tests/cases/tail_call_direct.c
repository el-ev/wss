// tail_call_direct.c
static volatile int g_seed = 17;

__attribute__((noinline)) static int leaf(int x) { return (x * 5) - 9; }

__attribute__((noinline)) static int forward_two(int x) { return leaf(x + 4); }

__attribute__((noinline)) static int forward_one(int x) { return forward_two(x); }

int _start(void) {
  int seed = g_seed;
  return forward_one(seed); // 96 (0x60)
}
