int _start(void) {
  volatile unsigned short *p = (volatile unsigned short *)(unsigned int)1;
  p[0] = (unsigned short)0xa1b2;
  return (unsigned short)p[0];
}
