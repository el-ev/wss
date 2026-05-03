// table_memory_stride_long.c
typedef int (*fn_t)(int, int);

static volatile unsigned seed = 0x00c0ffeeu;

__attribute__((noinline)) static int add2(int a, int b) { return a + b; }
__attribute__((noinline)) static int sub2(int a, int b) { return a - b; }
__attribute__((noinline)) static int mul2(int a, int b) { return a * b; }
__attribute__((noinline)) static int xor2(int a, int b) { return a ^ b; }

int _start(void) {
  fn_t table[4] = {add2, sub2, mul2, xor2};
  volatile unsigned char *buf = (volatile unsigned char *)(unsigned)0x340;
  unsigned acc = seed;

  for (unsigned i = 0; i < 12u; i++) {
    unsigned a = acc ^ (i * 13u);
    unsigned b = ((acc >> ((i & 3u) + 1u)) | 1u) + i;
    int v = table[(a + b) & 3u]((int)a, (int)b);

    buf[i & 7u] = (unsigned char)v;
    int s = (int)(signed char)buf[i & 7u];

    switch ((v + (int)i) & 3) {
      case 0:
        acc += (unsigned)(v ^ s);
        break;
      case 1:
        acc ^= (unsigned)(v - s);
        continue;
      case 2:
        acc -= (unsigned)(v + s);
        break;
      default:
        acc += (unsigned)((v << 1) ^ (int)i);
        break;
    }

    acc = (acc << 3) ^ (acc >> 2) ^ (unsigned)(s + (int)i);
  }

  seed = acc;
  return (int)acc;
}
