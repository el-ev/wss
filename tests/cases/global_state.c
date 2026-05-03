// global_state.c
static int g = 5;

static int bump(void) {
  g = g * 3 + 1;
  return g;
}

int _start(void) {
  int a = bump(); // 16
  int b = bump(); // 49
  return a + b + g;
}

