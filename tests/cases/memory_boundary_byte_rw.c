// memory_boundary_byte_rw.c
int _start(void) {
  volatile unsigned char *p = (volatile unsigned char *)(unsigned int)1023;
  p[0] = (unsigned char)0x5a;
  return (unsigned char)p[0];
}
