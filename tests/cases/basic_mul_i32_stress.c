typedef unsigned u32;
typedef int i32;

static volatile u32 ua0 = 0x12345678u;
static volatile u32 ub0 = 0x00fedcbau;
static volatile u32 ua1 = 0xf0000001u;
static volatile u32 ub1 = 0x00010003u;

static volatile i32 sa0 = -123456789;
static volatile i32 sa1 = 2000000000;

#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

int _start(void) {
  u32 acc = 0x6d2b79f5u;

  MIX(acc, ua0 * (ub0 | 1u));
  MIX(acc, ua1 * (ub1 | 1u));
  MIX(acc, (u32)(-sa0));
  MIX(acc, (u32)(-sa1));

  return (int)acc;
}
