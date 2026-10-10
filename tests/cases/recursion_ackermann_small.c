// recursion_ackermann_small.c: doubly-recursive Ackermann with nested call
// results feeding arguments. ack(2,3) = 9, ack(3,1) = 13.
static volatile int m0 = 2, n0 = 3, m1 = 3, n1 = 1;

__attribute__((noinline)) static int ack(int m, int n) {
  if (m == 0) {
    return n + 1;
  }
  if (n == 0) {
    return ack(m - 1, 1);
  }
  return ack(m - 1, ack(m, n - 1));
}

int _start(void) { return ack(m0, n0) * 16 + ack(m1, n1); } // 157
