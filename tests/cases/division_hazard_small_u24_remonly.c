// division_hazard_small_u24_remonly.c
int _start(void) {
  volatile unsigned int a = 0x00f12345u;
  volatile unsigned int b = 0x0000fedcu;
  unsigned int r = a % b;
  return (int)r;
}
