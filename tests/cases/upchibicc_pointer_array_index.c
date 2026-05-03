// upchibicc_pointer_array_index.c
// Upstream reference: rui314/chibicc@90d1f7f test/pointer.c
int _start(void) {
  int x[3];
  *x = 3;
  x[1] = 4;
  2[x] = 5;

  int y[2][3];
  int *p = (int *)y;
  p[0] = 0;
  p[1] = 1;
  p[2] = 2;
  p[3] = 3;
  p[4] = 4;
  p[5] = 5;

  int sum_x = x[0] + x[1] + x[2];
  int sum_y = y[0][0] + y[0][1] + y[0][2] + y[1][0] + y[1][1] + y[1][2];

  return sum_x * 10 + sum_y;
}
