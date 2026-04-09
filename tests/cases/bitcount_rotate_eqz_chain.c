static volatile unsigned seed = 0x89abcdefu;

__attribute__((noinline)) static int f1(int x) {
  return (x == 0) ? 17 : ((x & 31) - 13);
}

__attribute__((noinline)) static int f2(int x) {
  return (x < 0) ? (x >> 1) : (x << 1);
}

__attribute__((noinline)) static int f3(int x) {
  unsigned ux = (unsigned)x;
  unsigned rot = (ux >> 5) | (ux << 27);
  return (int)(rot ^ (ux >> 11) ^ __builtin_popcount(ux));
}

int _start(void) {
  int acc = 0;
  unsigned x = seed;

  for (int i = 0; i < 4; i++) {
    int a = f1((int)(x & 63u) - 31);
    int b = f2(a ^ i);
    int c = f3(b ^ (int)x);
    int is_zero = (c == 0) ? 1 : 0;

    acc += is_zero ? (b - c) : (b + c);
    x = (x >> 3) | (x << 29);
    x ^= (unsigned)(acc + i * 19);
  }

  seed = x;
  return acc ^ (int)x;
}
