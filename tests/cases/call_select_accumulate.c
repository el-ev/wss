// call_select_accumulate.c
static volatile int state = 37;

__attribute__((noinline)) static int f(int a) { return a * 3 - 5; }
__attribute__((noinline)) static int g(int a) { return (a ^ 0x55) + 7; }

int _start(void) {
  int acc = state;

  for (int i = 0; i < 4; i++) {
    int a = f(acc + i);
    int b = g(acc - i);
    int c = (a > b) ? (a - b) : (b - a);
    acc = ((i & 1) ? a : b) ^ c ^ acc;
  }

  state = acc;
  return acc;
}
