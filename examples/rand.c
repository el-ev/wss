extern int rand(void);
extern int putchar(int c);

static int hexd(unsigned v) {
  return v < 10u ? (int)('0' + v) : (int)('a' + v - 10u);
}

static void print_uint(unsigned n) {
  if (n >= 10u)
    print_uint(n / 10u);
  putchar((int)('0' + (n % 10u)));
}

int _start(void) {
  unsigned hist[16] = {0};
  for (int i = 0; i < 256; i++) {
    unsigned v = (unsigned)rand();
    unsigned d = v & 15u;
    hist[d]++;
    putchar(hexd(d));
    if ((i & 63) == 63)
      putchar('\n');
  }
  putchar('\n');
  for (int b = 0; b < 16; b++) {
    putchar(hexd((unsigned)b));
    putchar(':');
    putchar(' ');
    print_uint(hist[b]);
    putchar('\n');
  }
  return 0;
}
