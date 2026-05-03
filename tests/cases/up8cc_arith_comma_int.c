// up8cc_arith_comma_int.c
// Upstream references:
// - rui314/8cc@b480958 test/arith.c (test_comma)
// - rui314/chibicc@90d1f7f test/control.c (comma/lvalue comma expression)
int _start(void) {
  int total = 0;
  int i = 2;
  int j = 3;
  int a = 0;

  total += (1, 3);
  total += (a = 5, a);
  total += (a = 7, a + 1);
  total += (i = 5, j = 6, i + j);
  total += (j = 9, j);

  return total;
}
