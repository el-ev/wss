static volatile int seed_a = -3;
static volatile int seed_b = 2;

int _start(void) {
  int a = seed_a;
  int b = seed_b;
  unsigned int ua = (unsigned int)a;
  unsigned int ub = 2u;

  int mask = 0;
  if (a < b) mask |= 1;
  if (a > b) mask |= 2;
  if (ua < ub) mask |= 4;
  if (ua > ub) mask |= 8;
  if (a == -3) mask |= 16;
  if (ua != ub) mask |= 32;
  return mask; // 57 (0x39)
}
