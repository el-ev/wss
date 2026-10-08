// initial_store_after_call.c
// `g = 0` writes g's initial value, so it is a no-op on the first
// iteration only: every later iteration follows a call that changed g.
extern int putchar(int c);

volatile int g;

__attribute__((noinline)) void bump(void) { g = g + 5; }

int _start(void) {
  for (int i = 0; i < 3; i++) {
    g = 0;
    bump();
    putchar('0' + g);
  }
  putchar('\n');
  return 0x2a;
}
