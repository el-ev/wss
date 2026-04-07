int _start(void) {
  volatile unsigned short *p = (volatile unsigned short *)(unsigned int)1022;
  p[0] = (unsigned short)0xbeef;
  return (unsigned short)p[0];
}
