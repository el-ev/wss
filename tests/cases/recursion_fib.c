__attribute__((noinline)) static int fib(int n) {
  if (n < 2)
    return n;
  return fib(n - 1) + fib(n - 2);
}

int _start(void) {
  // Tiny recursion case to keep runtime practical in the blackbox suite.
  return fib(4); // 3 (0x03)
}
