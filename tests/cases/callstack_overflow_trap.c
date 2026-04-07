__attribute__((noinline)) static int dive_b(int n);

__attribute__((noinline)) static int dive_a(int n) {
  if (n == 0)
    return 1;
  return 1 + dive_b(n - 1);
}

__attribute__((noinline)) static int dive_b(int n) {
  if (n == 0)
    return 1;
  return 1 + dive_a(n - 1);
}

int _start(void) {
  // Mutual recursion helps keep calls in the generated Wasm.
  // This should exceed the CSS VM callstack budget before reaching base case.
  return dive_a(600);
}
