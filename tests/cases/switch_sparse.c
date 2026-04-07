static int map_sparse(int x) {
  switch (x) {
  case -100:
    return 7;
  case 0:
    return 11;
  case 100:
    return 13;
  case 1000:
    return 17;
  default:
    return 19;
  }
}

int _start(void) {
  int acc = 0;
  acc += map_sparse(-100);
  acc += map_sparse(0);
  acc += map_sparse(100);
  acc += map_sparse(1000);
  acc += map_sparse(42);
  return acc;
}
