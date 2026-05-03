// division_remainder.c
int _start(void) {
  int a = 100 / 7;               // 14
  int b = 100 % 7;               // 2
  unsigned int c = 100u / 9u;    // 11
  unsigned int d = 100u % 9u;    // 1
  return a + b + (int)c + (int)d;
}

