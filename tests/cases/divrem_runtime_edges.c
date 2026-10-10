// divrem_runtime_edges.c: variable-divisor div/rem over edge operands
// (sign boundaries, divisors with the top bit set, quotients near 2^32).
typedef unsigned u32;
typedef int i32;

static volatile u32 nums[] = {
    0u, 1u, 7u, 255u, 256u, 65535u, 65536u, 16777215u, 16777216u,
    0x7fffffffu, 0x80000000u, 0x80000001u, 0xfffffffeu, 0xffffffffu,
    0x12345678u, 0xdeadbeefu, 4294967291u, 2863311530u,
};
static volatile u32 dens[] = {
    1u, 2u, 3u, 7u, 255u, 256u, 257u, 65535u, 65537u, 16777217u,
    0x7fffffffu, 0x80000000u, 0xfffffffeu, 0xffffffffu, 0xfffffff9u,
    4294967291u, 0x10000u, 1431655765u,
};

#define N (sizeof(nums) / sizeof(nums[0]))
#define D (sizeof(dens) / sizeof(dens[0]))
#define MIX(acc, v) ((acc) = (((acc) ^ (u32)(v)) * 1664525u + 1013904223u))

int _start(void) {
  u32 acc = 0x6d2b79f5u;
  for (u32 i = 0; i < N; i++) {
    for (u32 j = 0; j < D; j++) {
      u32 a = nums[i], b = dens[j];
      MIX(acc, a / b);
      MIX(acc, a % b);
      if (!((i32)a == (i32)0x80000000 && (i32)b == -1)) {
        MIX(acc, (i32)a / (i32)b);
        MIX(acc, (i32)a % (i32)b);
      }
    }
  }
  return (int)acc;
}
