// indirect_halfword_guard.c
typedef int (*unop_t)(int);

static volatile int g = 0x55aa33cc;

__attribute__((noinline)) static int bump(int x) { return x + 9; }
__attribute__((noinline)) static int fold(int x) { return (x ^ 0x00001357) - 4; }

int _start(void) {
  unop_t table[2] = {bump, fold};
  volatile unsigned short *mem = (volatile unsigned short *)(unsigned)0x220;
  int acc = 0;

  for (int i = 0; i < 3; i++) {
    int x = g ^ (i * 17);
    int y = table[(x >> 1) & 1](x);

    mem[i] = (unsigned short)y;
    int z = (int)(short)mem[i];
    acc += (z >= 0) ? z : -z;
    g = g + (z ^ i);
  }

  return acc ^ g;
}
