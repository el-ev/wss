// basic_relops_i32_stress.c
typedef unsigned u32;
typedef int i32;

static volatile i32 sa0 = -17;
static volatile i32 sb0 = 9;
static volatile i32 sa1 = 123456789;
static volatile i32 sb1 = 123456789;

#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

#define STEP_S(acc, a, b)                                                     \
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

  STEP_S(acc, sa0, sb0);
  STEP_S(acc, sa1, sb1);

  return (int)acc;
}
