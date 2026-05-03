// basic_relops_u32_stress.c
typedef unsigned u32;

static volatile u32 ua0 = 0x00000000u;
static volatile u32 ub0 = 0x00000000u;
static volatile u32 ua1 = 0xf0000000u;
static volatile u32 ub1 = 0x0ffffffau;

#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

#define STEP_U(acc, a, b)                                                     \
  do {                                                                        \
    MIX(acc, (u32)((a) == (b)));                                              \
    MIX(acc, (u32)((a) != (b)));                                              \
    MIX(acc, (u32)((a) < (b)));                                               \
    MIX(acc, (u32)((a) <= (b)));                                              \
    MIX(acc, (u32)((a) > (b)));                                               \
    MIX(acc, (u32)((a) >= (b)));                                              \
    MIX(acc, (u32)!(a));                                                      \
    MIX(acc, (u32)((a) && (b)));                                              \
    MIX(acc, (u32)((a) || (b)));                                              \
  } while (0)

int _start(void) {
  u32 acc = 0x6d2b79f5u;

  STEP_U(acc, ua0, ub0);
  STEP_U(acc, ua1, ub1);

  return (int)acc;
}
