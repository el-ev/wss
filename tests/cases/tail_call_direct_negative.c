__attribute__((noinline)) static int leaf(int x) { return x - 100; }

__attribute__((noinline)) static int hop_two(int x) { return leaf(x * 3); }

__attribute__((noinline)) static int hop_one(int x) { return hop_two(x + 5); }

int _start(void) {
  return hop_one(7); // (7 + 5) * 3 - 100 = -64 (0xffffffc0)
}
