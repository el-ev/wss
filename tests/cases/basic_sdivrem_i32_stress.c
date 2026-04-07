typedef unsigned u32;
typedef int i32;

static volatile i32 sa0 = -123456789;
static volatile i32 sb0 = 37;

#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

int _start(void) {
  u32 acc = 0x6d2b79f5u;

  MIX(acc, (u32)(sa0 / sb0));
  MIX(acc, (u32)(sa0 % sb0));

  return (int)acc;
}
