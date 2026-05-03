// invalid_memory_edge_store_trap.c
int _start(void) {
  volatile int *p = (int *)(unsigned int)1021;
  *p = 123;
  return 0;
}
