static volatile unsigned seed = 0x12345678u;

int _start(void) {
  unsigned x = seed;
  return __builtin_clz(x) + __builtin_ctz(x) + __builtin_popcount(x);
}
