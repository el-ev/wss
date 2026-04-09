static volatile unsigned seed = 0x13579bdfu;

__attribute__((optnone, noinline)) static int walk(unsigned x) {
  int acc = 0;

  for (unsigned i = 0; i < 5; i++) {
    unsigned lane = (x + i * 3u) & 7u;

    switch (lane) {
      case 0:
        acc += (int)i;
        break;
      case 1:
        acc ^= (int)(i << 2);
        continue;
      case 2:
        acc -= (int)(i * 5u);
        break;
      case 3:
        acc += __builtin_popcount(x ^ i);
        break;
      case 4:
        acc += (int)((x >> (i & 7u)) & 31u);
        break;
      case 5:
        acc -= (int)((x << (i & 3u)) >> 29);
        break;
      case 6:
        if ((acc & 1) != 0) {
          acc += 11;
          break;
        }
        acc -= 7;
        continue;
      default:
        acc ^= (int)((x >> (i & 7u)) | (x << ((8u - i) & 7u)));
        break;
    }

    x = (x << 1) ^ (unsigned)acc;
  }

  return acc ^ (int)x;
}

int _start(void) {
  unsigned x = seed;
  seed = x + 1u;
  return walk(x);
}
