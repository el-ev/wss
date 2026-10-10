// divrem_const_signed_negative.c: signed div/rem by non-power-of-two
// constants (positive and negative) over negative dividends and the
// INT_MIN / INT_MAX extremes. Constant divisors lower to magic-multiply
// sequences whose sign fixups must round toward zero.
typedef unsigned u32;
typedef int i32;

static volatile i32 vals[] = {-1, -7, -100, -2147483647 - 1, 2147483647, 1000};

#define ROTMIX(acc, v) ((acc) = (((acc) << 5) | ((acc) >> 27)) ^ (u32)(v))

int _start(void) {
  u32 acc = 0x12345u;
  for (u32 i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
    i32 s = vals[i];
    ROTMIX(acc, s / 3);
    ROTMIX(acc, s % 3);
    ROTMIX(acc, s / -5);
    ROTMIX(acc, s % -5);
    ROTMIX(acc, s / 255);
    ROTMIX(acc, s % 1000);
  }
  return (int)acc;
}
