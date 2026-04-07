int _start(void) {
  int sum = 0;

  for (int i = 0; i < 20; i++) {
    if ((i & 1) == 0) {
      continue;
    }
    if (i > 11) {
      break;
    }
    sum += i * 3;
  }

  int j = 0;
  do {
    sum += j * 2 + 1;
    j += 1;
  } while (j < 4);

  return sum; // 108 + 16 = 124 (0x7c)
}
