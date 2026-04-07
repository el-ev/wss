// Upstream reference: rui314/chibicc@90d1f7f test/control.c
static volatile int g_limit = 10;

static int loop_break_value(void) {
  int i = 0;
  for (; i < g_limit; i++) {
    if (i == 3)
      break;
  }
  return i;
}

static int loop_continue_for_value(void) {
  int i = 0;
  int j = 0;
  for (; i < g_limit; i++) {
    if (i > 5)
      continue;
    j++;
  }
  return j;
}

static int loop_continue_while_value(void) {
  int i = 0;
  int j = 0;
  while (i++ < g_limit) {
    if (i > 5)
      continue;
    j++;
  }
  return j;
}

int _start(void) {
  int a = loop_break_value();
  int b = loop_continue_for_value();
  int c = loop_continue_while_value();
  return a * 100 + b * 10 + c;
}
