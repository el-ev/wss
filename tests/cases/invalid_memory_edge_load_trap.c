// invalid_memory_edge_load_trap.c
int _start(void) {
  volatile int *p = (int *)(unsigned int)1021;
  return *p;
}
