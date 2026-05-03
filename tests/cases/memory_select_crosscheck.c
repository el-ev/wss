// memory_select_crosscheck.c
static volatile unsigned base = 0x10293847u;

int _start(void) {
  volatile unsigned char *bytes = (volatile unsigned char *)(unsigned)0x260;
  volatile unsigned short *halfs = (volatile unsigned short *)(unsigned)0x280;
  int acc = 0;
  unsigned x = base;

  for (int i = 0; i < 3; i++) {
    unsigned y = (x >> (i * 3)) ^ (x << (i + 1));
    bytes[i] = (unsigned char)y;
    halfs[i] = (unsigned short)(y ^ (x >> 16));

    int sb = (int)(signed char)bytes[i];
    int sh = (int)(short)halfs[i];
    acc += (sb > 0) ? (sh + sb) : (sh - sb);

    x = y ^ (unsigned)(acc + i);
  }

  base = x;
  return acc ^ (int)x;
}
