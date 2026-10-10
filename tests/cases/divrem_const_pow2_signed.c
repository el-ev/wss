// divrem_const_pow2_signed.c: signed and unsigned div/rem by powers of two.
// Signed variants lower to arithmetic shifts with a bias fixup for negative
// dividends; the remainder must keep the dividend's sign.
typedef unsigned u32;
typedef int i32;

static volatile i32 vals[] = {-1,         -2,   -3,  -255, -256, -257,
                              0x7fffffff, -2147483647 - 1, 5, 1024};

#define ROTMIX(acc, v) ((acc) = (((acc) << 5) | ((acc) >> 27)) ^ (u32)(v))

int _start(void) {
  u32 acc = 7u;
  for (u32 i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
    i32 s = vals[i];
    u32 u = (u32)s;
    ROTMIX(acc, s / 2);
    ROTMIX(acc, s % 2);
    ROTMIX(acc, s / 16);
    ROTMIX(acc, s % 16);
    ROTMIX(acc, s / 256);
    ROTMIX(acc, s % 256);
    ROTMIX(acc, s / 65536);
    ROTMIX(acc, s % (1 << 30));
    ROTMIX(acc, u / 8);
    ROTMIX(acc, u % 512);
  }
  return (int)acc;
}
