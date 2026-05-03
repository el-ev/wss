// invalid_memory_halfword_edge_trap.c
int _start(void) {
  volatile unsigned short *p = (volatile unsigned short *)(unsigned int)1023;
  return (unsigned short)p[0];
}
