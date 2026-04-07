int _start(void) {
  volatile int *base = (volatile int *)0x60;
  for (int i = 0; i < 6; i++) {
    base[i] = i * i - 3 * i + 7;
  }

  volatile int *p = base + 1;
  p[2] = p[2] + base[0];

  int acc = 0;
  for (int i = 0; i < 6; i++) {
    acc = (acc << 1) ^ base[i];
  }

  return acc; // 167 (0xa7)
}
