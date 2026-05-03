// up8cc_arith_ternary_elvis.c
// Upstream reference: rui314/8cc@b480958 test/arith.c (test_ternary)
static volatile int g_zero = 0;

int _start(void) {
  int sum = 0;

  sum += (g_zero + 1 + 2) ? 51 : 52;
  sum += (g_zero + 1 - 1) ? 51 : 52;
  sum += (g_zero + 1 - 1) ? 51 : (52 / 2);
  sum += (g_zero + 1 - 0) ? (51 / 3) : 52;

  // GNU extension
  sum += g_zero ?: 52;
  sum += (g_zero + 1 + 2) ?: 52;

  return sum;
}
