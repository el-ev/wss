// loop_break_continue_do_while.c
static volatile int odd_limit = 20;
static volatile int break_limit = 11;
static volatile int do_limit = 4;

int _start(void) {
  int sum = 0;

  for (int i = 0; i < odd_limit; i++) {
    if ((i & 1) == 0) {
      continue;
    }
    if (i > break_limit) {
      break;
    }
    sum += i * 3;
  }

  int j = 0;
  do {
    sum += j * 2 + 1;
    j += 1;
  } while (j < do_limit);

  return sum; // 108 + 16 = 124 (0x7c)
}
