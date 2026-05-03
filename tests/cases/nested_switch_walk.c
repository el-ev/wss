// nested_switch_walk.c
static volatile unsigned seed10 = 0x2au;

__attribute__((optnone, noinline)) static int walk(unsigned s) {
  int acc = 1;
  for (unsigned i = 0; i < 7; i++) {
    for (unsigned j = 0; j < 4; j++) {
      unsigned k = (s + i * 3u + j * 5u) & 7u;
      switch (k) {
        case 0:
          acc += (int)(i + j);
          break;
        case 1:
          acc -= (int)(i * 2u + j);
          break;
        case 2:
          acc ^= (int)(k << (j & 1u));
          break;
        case 3:
          if ((acc & 1) == 0)
            continue;
          acc += 9;
          break;
        case 4:
          if (acc > 200)
            break;
          acc += 11;
          break;
        case 5:
          acc -= 13;
          break;
        default:
          acc += (int)k;
          break;
      }
      if (acc < -300)
        return acc;
    }
  }
  return acc ^ (int)s;
}

int _start(void) {
  unsigned s = seed10;
  seed10 = s ^ 0x77u;
  return walk(s);
}
