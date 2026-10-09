// divrem_const_i32.c
// Runtime dividends against compile-time-constant divisors, which lower to
// inline per-byte div/rem expressions rather than the shared helper.
typedef unsigned u32;
typedef int i32;

static volatile u32 vals[] = {
    0u,          1u,          9u,          10u,         623u,
    624u,        65535u,      123456789u,  0x7fffffffu, 0x80000000u,
    0x80000001u, 0xfffffff6u, 0xfedcba98u, 0xffffffffu,
};

#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

int _start(void) {
  u32 acc = 0x6d2b79f5u;
  for (u32 i = 0; i < sizeof(vals) / sizeof(vals[0]); ++i) {
    u32 u = vals[i];
    i32 s = (i32)u;
    MIX(acc, u / 3u);
    MIX(acc, u % 3u);
    MIX(acc, u / 10u);
    MIX(acc, u % 10u);
    MIX(acc, u / 624u);
    MIX(acc, u % 624u);
    MIX(acc, u / 65537u);
    MIX(acc, u % 65537u);
    MIX(acc, u / 0x01000001u);
    MIX(acc, u % 0x01000001u);
    MIX(acc, u / 0xfffffff6u);
    MIX(acc, u % 0xfffffff6u);
    MIX(acc, s / 7);
    MIX(acc, s % 7);
    MIX(acc, s / -10);
    MIX(acc, s % -10);
    MIX(acc, s / 1000000);
    MIX(acc, s % 1000000);
    MIX(acc, s / (i32)0x80000000u);
    MIX(acc, s % (i32)0x80000000u);
  }
  return (int)acc;
}
