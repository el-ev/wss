typedef unsigned u32;

static volatile u32 ua0 = 0x12345678u;
static volatile u32 ub0 = 0x00fedcbau;

#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

int _start(void) {
  u32 acc = 0x6d2b79f5u;
  u32 d = (ub0 & 0x0fffffffu) | 1u;

  MIX(acc, ua0 / d);
  MIX(acc, ua0 % d);

  return (int)acc;
}
