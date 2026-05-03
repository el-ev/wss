// io_triplet_transform.c
extern int putchar(int c);
extern int getchar(void);

static volatile unsigned seed = 0x31u;

__attribute__((optnone, noinline)) static int twist(int ch, int i, int acc) {
  unsigned x = (unsigned)(ch ^ acc ^ (int)seed);
  unsigned s = ((unsigned)i + seed) & 7u;
  unsigned r = (x << s) | (x >> (8u - s));
  return (int)(r & 0xffu);
}

int _start(void) {
  int acc = (int)seed;

  for (int i = 0; i < 3; i++) {
    int ch = getchar();
    int t = twist(ch, i, acc);
    int out = (ch >= 'a' && ch <= 'z')
                  ? (ch - 32)
                  : ((ch >= 'A' && ch <= 'Z') ? (ch + 32) : ch);

    putchar(out);
    acc = ((acc << 2) ^ t) + out;
  }

  seed = (unsigned)acc;
  return acc;
}
