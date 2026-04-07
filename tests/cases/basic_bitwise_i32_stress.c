typedef unsigned u32;
typedef int i32;

static volatile u32 ua0 = 0x12345678u;
static volatile u32 ub0 = 0x00fedcbau;
static volatile u32 us0 = 5u;
static volatile u32 ua1 = 0xf0f0aa55u;
static volatile u32 ub1 = 0x0f3355aau;
static volatile u32 us1 = 13u;

static volatile i32 sa0 = -123456789;
static volatile u32 ss0 = 3u;
static volatile i32 sa1 = -2004318072;
static volatile u32 ss1 = 7u;

#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

#define STEP_U(acc, a, b, sh_src)                                             \
  do {                                                                        \
    u32 sh = (sh_src) & 31u;                                                  \
    MIX(acc, (a) & (b));                                                      \
    MIX(acc, (a) | (b));                                                      \
    MIX(acc, (a) ^ (b));                                                      \
    MIX(acc, ~(a));                                                           \
    MIX(acc, (a) << sh);                                                      \
    MIX(acc, (a) >> sh);                                                      \
  } while (0)

#define STEP_S(acc, a, sh_src)                                                \
  do {                                                                        \
    u32 sh = (sh_src) & 31u;                                                  \
    MIX(acc, (u32)((a) >> sh));                                               \
  } while (0)

int _start(void) {
  u32 acc = 0x6d2b79f5u;

  STEP_U(acc, ua0, ub0, us0);
  STEP_U(acc, ua1, ub1, us1);

  STEP_S(acc, sa0, ss0);
  STEP_S(acc, sa1, ss1);

  return (int)acc;
}
