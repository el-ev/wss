// global_set_stack_frame.c
__attribute__((noinline)) static int sink(int x) { return x + 3; }

int _start(void) {
  volatile int buf[8];
  buf[0] = 20;
  int y = sink(buf[0]);
  buf[7] = y;
  return buf[7] + buf[0];
}
