int _start(void) {
  int a = 13;
  int b = 29;
  int c = 13;
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
