// upchibicc_initializer_designated.c
// Upstream reference: rui314/chibicc@90d1f7f test/initializer.c
int _start(void) {
  int a[3] = {1, 2, 3, [0] = 4, 5};
  struct Pair {
    int x;
    int y;
  } p = {1, 2, .y = 3, .x = 4};

  int m[2][3] = {
      1, 2, 3, 4, 5, 6, [0][1] = 7, 8, [0] = 9, [0] = 10, 11, [1][0] = 12,
  };

  int sum = 0;
  sum += a[0] + a[1] + a[2];
  sum += p.x + p.y;
  sum += m[0][0] + m[0][1] + m[0][2];
  sum += m[1][0] + m[1][1] + m[1][2];
  return sum;
}
