static volatile unsigned char buf[16];

int _start(void) {
  buf[0] = 0x80u;
  buf[1] = 0x7fu;
  ((volatile unsigned short *)(void *)&buf[2])[0] = 0x1234u;
  ((volatile unsigned short *)(void *)&buf[4])[0] = 0xff80u;

  int a = (signed char)buf[0];
  int b = (unsigned char)buf[1];
  int c = (signed short)((volatile unsigned short *)(void *)&buf[2])[0];
  int d = (signed short)((volatile unsigned short *)(void *)&buf[4])[0];
  int e = (int)((volatile unsigned short *)(void *)&buf[2])[0];

  return (a + b + c + d + e) ^ 0x1357;
}
