// Upstream reference: rui314/8cc@b480958 test/arith.c (test_inc_dec)
static volatile int g_seed = 15;

int _start(void) {
  int a = g_seed;
  int sum = 0;

  sum += a++;
  sum += a;
  sum += a--;
  sum += a;
  sum += --a;
  sum += a;
  sum += ++a;
  sum += a;

  return sum;
}
