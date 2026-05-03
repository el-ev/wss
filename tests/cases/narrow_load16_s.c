// narrow_load16_s.c
int _start(void) {
  volatile unsigned short *p16 = (volatile unsigned short *)0x40;
  p16[0] = (unsigned short)0x8001;
  return (short)p16[0];
}
