// builtin_popcount_seed.c
static volatile unsigned seed = 0x12345678u;

int _start(void) {
  unsigned x = seed;
  return __builtin_popcount(x);
}
