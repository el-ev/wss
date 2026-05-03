// bitcount_relop_blend.c
static volatile unsigned va = 0x01020304u;
static volatile unsigned vb = 0x80010001u;

__attribute__((noinline, optnone)) static int score(unsigned x, unsigned y) {
  int s = __builtin_clz(x) + __builtin_ctz(y) + __builtin_popcount(x ^ y);

  if ((int)x > (int)y)
    s += 3;
  else
    s += 5;

  if (x < y)
    s += 7;
  else
    s += 11;

  s += ((x & 255u) == (y & 255u)) ? 13 : 17;
  s += (int)((x >> 3) | (y << 2));
  return s;
}

int _start(void) {
  unsigned x = va;
  unsigned y = vb;
  va = y ^ 0x1020u;
  vb = x ^ 0x3344u;
  return score(x, y);
}
