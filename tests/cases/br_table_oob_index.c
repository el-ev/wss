// br_table_oob_index.c: a dense switch (br_table) driven by indices that
// fall outside the table on both sides, including INT_MIN and -1, which
// must hit the default arm via the unsigned range check.
static volatile int idx[] = {0, 3, 7, 8, -1, 100, (int)0x80000000, 5, 2, -2147483647};

__attribute__((noinline)) static int dispatch(int k) {
  switch (k) {
  case 0:
    return k + 11;
  case 1:
    return k * 23;
  case 2:
    return k ^ 37;
  case 3:
    return k - 41;
  case 4:
    return k << 3;
  case 5:
    return k | 61;
  case 6:
    return k & 73;
  case 7:
    return 89 - k;
  default:
    return (k >> 1) - 5;
  }
}

int _start(void) {
  int s = 0;
  for (int i = 0; i < 10; i++) {
    s = s * 17 + dispatch(idx[i]);
  }
  return s;
}
