// division_large_signed.c
int _start(void) {
  volatile int c = -2000000001;
  volatile int d = 34567;

  int q2 = c / d;

  unsigned int mix = (unsigned int)q2 ^ 0x013579bdu;
  return (int)mix;
}
