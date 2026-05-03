// loop_if_else.c
static volatile int limit = 10;

int _start(void) {
  int sum = 0;
  for (int i = 1; i <= limit; i++) {
    if ((i & 1) == 0) {
      sum += i * 2;
    } else {
      sum += i;
    }
  }
  return sum; // 85 (0x55)
}
