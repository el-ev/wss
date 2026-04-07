int _start(void) {
  volatile int c = -2000000001;
  volatile int d = 34567;
  int q2 = c / d;
  return q2;
}
