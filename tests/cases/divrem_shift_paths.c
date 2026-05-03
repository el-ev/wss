// divrem_shift_paths.c
static volatile int sa = -321;
static volatile int sb = 17;
static volatile unsigned ua = 123456789u;
static volatile unsigned ub = 97u;

int _start(void) {
  int a = sa;
  int b = sb;
  unsigned x = ua;
  unsigned y = ub;

  sa = a + 1;
  ua = x ^ 0x55aa55aau;

  int r = (a / b) + (a % b);
  r += (int)(x / y);
  r += (int)(x % y);
  r ^= (a >> 2);
  r += (int)(x << 3);
  return r;
}
