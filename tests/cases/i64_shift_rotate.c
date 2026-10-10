// i64_shift_rotate.c: 64-bit shifts and rotates by runtime amounts that
// straddle the 32-bit half boundary (31, 32, 33) and exceed 64 (masked).
static volatile unsigned long long x = 0x8000000000000001ull;
static volatile unsigned long long y = 0x0123456789abcdefull;
static volatile int amts[] = {0, 1, 31, 32, 33, 63, 64, 65, -1, 100};

long long _start(void) {
  unsigned long long acc = 0;
  for (int i = 0; i < 10; i++) {
    unsigned s = (unsigned)amts[i] & 63u;
    acc = (acc << 1) ^ (x << s) ^ (y >> s);
    acc ^= (unsigned long long)((long long)x >> s);
    acc ^= __builtin_rotateleft64(y, (unsigned)amts[i]);
    acc ^= __builtin_rotateright64(x, (unsigned)amts[i]);
  }
  return (long long)acc;
}
