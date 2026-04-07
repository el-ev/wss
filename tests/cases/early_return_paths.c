static volatile int g = 27;

__attribute__((noinline, optnone)) static int classify(int x) {
  if (x < -10)
    return x + 111;
  if ((x & 1) == 0)
    return (x * 3) - 5;
  if (x > 30)
    return x ^ 0x55;
  return (x << 2) + (x >> 1);
}

int _start(void) {
  int x = g;
  int a = classify(x - 40);
  int b = classify(x);
  int c = classify(x + 19);
  g = x ^ 0x33;
  return a + b + c;
}
