// control_switch_weave.c
static volatile int seed = 19;

__attribute__((optnone, noinline)) static int step(int x) {
  int acc = 3;
  for (int i = 0; i < 6; i++) {
    int t = (x + i * 5) ^ (i << 3);
    if ((t & 1) != 0) {
      acc += t >> 1;
    } else {
      acc -= t << 1;
    }

    switch ((t ^ x) & 3) {
      case 0:
        acc += 7;
        break;
      case 1:
        acc ^= 11;
        break;
      case 2:
        acc -= 13;
        break;
      default:
        acc += (acc < 0) ? 17 : 19;
        break;
    }
  }
  return acc;
}

int _start(void) {
  int x = seed;
  seed = x ^ 0x55;
  return step(x) ^ step(x + 2);
}
