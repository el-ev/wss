// invalid_memory_load_trap.c
int _start(void) {
  volatile int *ptr = (int *)(unsigned int)4096;
  return *ptr;
}

