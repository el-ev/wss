// indirect_memory_rotmix.c
typedef int (*op_t)(int, int);

static volatile unsigned seed = 0x2468ace1u;

__attribute__((noinline)) static int op_add(int a, int b) { return a + b; }
__attribute__((noinline)) static int op_sub(int a, int b) { return a - b; }
__attribute__((noinline)) static int op_xor(int a, int b) { return a ^ b; }

__attribute__((noinline)) static int op_mix(int a, int b) {
  unsigned ua = (unsigned)a;
  unsigned ub = (unsigned)b;
  unsigned s = ub & 31u;
  unsigned rot = (ua << s) | (ua >> ((32u - s) & 31u));
  return (int)(rot ^ (ua + ub));
}

int _start(void) {
  op_t table[4] = {op_add, op_sub, op_xor, op_mix};
  volatile unsigned char *bytes = (volatile unsigned char *)(unsigned)0x140;
  volatile unsigned short *halfs = (volatile unsigned short *)(unsigned)0x180;
  unsigned x = seed;
  int acc = 0;

  for (unsigned i = 0; i < 4; i++) {
    unsigned idx = (x + i) & 3u;
    int v = table[idx]((int)(x ^ (i * 17u)), (int)(9u + i));

    bytes[i] = (unsigned char)v;
    halfs[i & 1u] = (unsigned short)(v ^ (int)(x >> i));

    int b = (int)(signed char)bytes[i];
    int h = (int)(short)halfs[i & 1u];
    acc += (b < 0) ? (h - b) : (h + b);

    x = (x << 1) ^ (unsigned)(acc + (int)i);
  }

  seed = x;
  return acc ^ (int)x;
}
