int _start(void) {
  volatile unsigned char *p8 = (volatile unsigned char *)0x20;
  p8[0] = (unsigned char)0x80;
  return (unsigned char)p8[0];
}
