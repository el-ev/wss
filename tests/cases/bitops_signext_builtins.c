static volatile unsigned seed = 0x12345678u;

int _start(void) {
  unsigned x = seed;
  unsigned s = (seed & 31u) | 1u;
  unsigned r1 = x >> s;
  int r2 = ((int)x) >> s;
  unsigned rotl = (x << s) | (x >> (32u - s));
  unsigned rotr = (x >> s) | (x << (32u - s));
  int a = __builtin_clz(x) + __builtin_ctz(x) + __builtin_popcount(x);
  signed char c = (signed char)seed;
  short h = (short)(seed ^ 0x7FFFu);
  int e = (int)c + (int)h;

  seed = x ^ rotl ^ rotr;
  return (int)(r1 ^ (unsigned)r2 ^ rotl ^ rotr) + a + e;
}
