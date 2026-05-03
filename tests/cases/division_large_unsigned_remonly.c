// division_large_unsigned_remonly.c
int _start(void) {
  volatile unsigned int a = 0xf1234567u;
  volatile unsigned int b = 0x00fedcbau;
  unsigned int r1 = a % b;
  return (int)r1;
}
