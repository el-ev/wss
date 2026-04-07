int _start(void) {
  volatile int *ptr = (int *)(unsigned int)4096;
  *ptr = 1234;
  return 7;
}

