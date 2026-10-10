// i64_bitcount_extend.c: clz/ctz/popcnt on 64-bit values whose interesting
// bits sit in either half, plus narrow-to-64 sign/zero extensions.
static volatile unsigned long long v[] = {0ull, 1ull, 0x80000000ull, 0x100000000ull,
                                          0x8000000000000000ull, 0x00ff00ff00ff00ffull};
static volatile signed char sc = -5;
static volatile short ss = -300;
static volatile int si = -70000;
static volatile unsigned ui = 0xfffffff0u;

long long _start(void) {
  unsigned long long acc = 0;
  for (int i = 0; i < 6; i++) {
    unsigned long long w = v[i];
    unsigned clz = w ? (unsigned)__builtin_clzll(w) : 64u;
    unsigned ctz = w ? (unsigned)__builtin_ctzll(w) : 64u;
    unsigned pop = (unsigned)__builtin_popcountll(w);
    acc = (acc << 9) ^ clz ^ ((unsigned long long)ctz << 7) ^ ((unsigned long long)pop << 14);
  }
  acc ^= (unsigned long long)(long long)sc;
  acc += (unsigned long long)(long long)ss << 16;
  acc ^= (unsigned long long)(long long)si;
  acc -= (unsigned long long)ui;
  return (long long)acc;
}
