// division_large_signed_remonly.c
int _start(void) {
  volatile int c = -2000000001;
  volatile int d = 34567;
  int r2 = c % d;
  return r2;
}
