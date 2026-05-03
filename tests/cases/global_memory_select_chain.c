// global_memory_select_chain.c
static volatile int g0 = 0x00102030;
static volatile int g1 = -77;

__attribute__((noinline)) static int blend(int a, int b, int c) {
  int t = (a < b) ? (b - a) : (a - b);
  return (c & 1) ? (t ^ c) : (t + c);
}

int _start(void) {
  volatile int *w = (volatile int *)(unsigned)0x1c0;
  volatile signed char *b = (volatile signed char *)(unsigned)0x1d0;
  volatile unsigned short *h = (volatile unsigned short *)(unsigned)0x1e0;

  int acc = 0;

  for (int i = 0; i < 4; i++) {
    int a = g0 + i * 9;
    int c = g1 - i * 3;
    int v = blend(a, c, i);

    w[i & 1] = v + acc;
    b[i] = (signed char)(v >> (i & 3));
    h[i & 1] = (unsigned short)(v ^ (int)((unsigned)a << 1));

    int r0 = w[i & 1];
    int r1 = (int)b[i];
    int r2 = (int)(short)h[i & 1];

    acc += (r0 >= 0) ? (r1 + r2) : (r1 - r2);

    g0 = g0 ^ (v + i);
    g1 = g1 + ((i & 1) ? -5 : 7);
  }

  return acc ^ g0 ^ g1;
}
