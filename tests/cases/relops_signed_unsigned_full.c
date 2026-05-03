// relops_signed_unsigned_full.c
static volatile unsigned ua = 0x80000010u;
static volatile unsigned ub = 0x0000fff0u;

__attribute__((optnone, noinline)) static int score(unsigned a, unsigned b) {
  int sa = (int)a;
  int sb = (int)b;
  int acc = 0;
  if (sa > sb)
    acc += 1;
  if (sa >= sb)
    acc += 2;
  if (sa < sb)
    acc += 4;
  if (sa <= sb)
    acc += 8;
  if (a > b)
    acc += 16;
  if (a >= b)
    acc += 32;
  if (a < b)
    acc += 64;
  if (a <= b)
    acc += 128;
  return acc;
}

int _start(void) {
  unsigned a = ua;
  unsigned b = ub;
  ua = b ^ 0x13579bdfu;
  ub = a ^ 0x2468ace0u;
  return score(a, b);
}
