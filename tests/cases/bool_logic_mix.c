// bool_logic_mix.c
static volatile int seed_a = 13;
static volatile int seed_b = 29;
static volatile int seed_c = 13;

int _start(void) {
  int a = seed_a;
  int b = seed_b;
  int c = seed_c;
  int mask = 0;

  if ((a == c && b > a) || (a > b))
    mask |= 1;
  if ((a != c) || (b < 0))
    mask |= 2;
  if (!(a == b))
    mask |= 4;
  if ((a + b) == 42 && (b - a) == 16)
    mask |= 8;
  if ((a ^ b) == 16)
    mask |= 16;
  if ((a & b) == 13)
    mask |= 32;

  return mask;
}
