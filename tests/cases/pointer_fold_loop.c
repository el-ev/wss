static volatile int arr[8] = {3, 5, 7, 11, 13, 17, 19, 23};

__attribute__((noinline, optnone)) static int fold(volatile int *p, int n) {
  int acc = 0;
  for (int i = 0; i < n; i++) {
    int v = p[i];
    if ((i & 1) == 0)
      acc += v * (i + 1);
    else
      acc ^= v << (i & 3);
    p[i] = v + i;
  }
  return acc;
}

int _start(void) {
  int r1 = fold(arr, 8);
  int r2 = fold(arr, 4);
  return r1 + r2 + arr[7];
}
