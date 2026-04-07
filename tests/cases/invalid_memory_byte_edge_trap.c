int _start(void) {
  volatile unsigned char *p = (volatile unsigned char *)(unsigned int)1024;
  return (unsigned char)p[0];
}
