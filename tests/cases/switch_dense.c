static int map_value(int x) {
  switch (x) {
  case 0:
    return 11;
  case 1:
    return 7;
  case 2:
    return 3;
  case 3:
    return 19;
  case 4:
    return 23;
  case 5:
    return 29;
  default:
    return 31;
  }
}

int _start(void) {
  int acc = 0;
  for (int i = -1; i <= 6; i++) {
    acc += map_value(i);
  }
  return acc; // 154 (0x9a)
}

