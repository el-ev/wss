static volatile unsigned seed = 0x6d5a1f23u;

int _start(void) {
  volatile unsigned char *bytes = (volatile unsigned char *)(unsigned)0x2a0;
  volatile unsigned short *halfs = (volatile unsigned short *)(unsigned)0x2c0;
  volatile unsigned *words = (volatile unsigned *)(unsigned)0x2e0;
  unsigned x = seed;
  int acc = 0;

  for (int i = 0; i < 3; i++) {
    unsigned t = (x ^ ((unsigned)i * 0x1111u)) + (x >> (i + 1));
    int sel = (t & 1u) ? (int)(t ^ 0x55aa00ffu) : (int)(t + 0x1234u);

    bytes[i] = (unsigned char)sel;
    halfs[i] = (unsigned short)(sel >> 3);
    words[i & 1] = (unsigned)sel ^ (x << 1);

    int sb = (int)(signed char)bytes[i];
    int sh = (int)(short)halfs[i];
    int sw = (int)words[i & 1];
    acc += (sw < 0) ? (sb - sh) : (sb + sh);

    x = (unsigned)(sw ^ acc) + (t << 1);
  }

  seed = x;
  return acc ^ (int)x;
}
